//! Hora - a tiny self-hosted uptime monitor.
//!
//! Wires the pieces together: load config, open the database, start the
//! supervisor (which owns the live config and notification channels), spawn the
//! certificate watcher and pruner, and serve the status page and JSON API.

use std::fmt::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use anyhow::Context as _;
use hora_core::announce::{self, Announcement};
use hora_core::silence::SilenceError;
use hora_core::{MAX_EVENT_TITLE_CHARS, config, fmt};

mod top;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // Plain `hora` (no arguments) starts the monitor; anything else is a
    // one-shot subcommand. Either way the outcome comes back here instead of a
    // `process::exit` deep inside, so destructors run (the SQLite pool closes,
    // the terminal is restored) before the process ends.
    let outcome = if args.len() > 1 {
        run_subcommand(&args).await
    } else {
        init_tracing();
        serve().await.map_err(CliError::from)
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => err.report(),
    }
}

/// Why a command ended unsuccessfully, turned into stderr output and an exit
/// code by [`CliError::report`].
#[derive(Debug)]
enum CliError {
    /// A malformed invocation (missing argument, bad flag value, unknown id):
    /// the message, usually a usage line, goes to stderr verbatim.
    Usage(String),
    /// The command ran and reached a negative verdict (delivery failed, a
    /// capability is missing, no such incident): the message goes to stderr
    /// verbatim.
    Failed(String),
    /// A negative verdict already printed on stdout (a down probe, peer
    /// drift): nothing more to say, only the exit code.
    Silent,
    /// Anything else (config, database, I/O), printed exactly as an error
    /// returned from `main` always was: `Error: ...` with its causes.
    Other(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for CliError {
    fn from(err: E) -> Self {
        Self::Other(err.into())
    }
}

impl CliError {
    /// Print the error and return the process exit code. Every failure exits
    /// 1, usage errors included: scripts already gate on that (`hora probe url
    /// && deploy`), so the conventional 2 for usage would be a breaking change.
    fn report(self) -> ExitCode {
        match self {
            Self::Usage(message) | Self::Failed(message) => eprintln!("{message}"),
            Self::Silent => {}
            Self::Other(err) => eprintln!("Error: {err:?}"),
        }
        ExitCode::FAILURE
    }
}

/// A [`CliError::Usage`] from any message.
fn usage(message: impl Into<String>) -> CliError {
    CliError::Usage(message.into())
}

/// Dispatch a CLI subcommand (`args[1]`; the caller checked it exists).
async fn run_subcommand(args: &[String]) -> Result<(), CliError> {
    let rest = &args[2..];
    match args[1].as_str() {
        "import" => import_kuma(rest),
        "check" => check_config(),
        "test-alert" => {
            // Tracing first: delivery failures surface as per-channel
            // warnings from the notifiers, and that is the whole point.
            init_tracing();
            test_alert(rest.first().map(String::as_str)).await
        }
        "backup" => {
            let dest = rest
                .first()
                .ok_or_else(|| usage("Usage: hora backup <destination.db>"))?;
            backup(dest).await
        }
        "incidents" => {
            let limit = parse_limit(rest.first(), "Usage: hora incidents [limit]")?;
            list_incidents(limit).await
        }
        "annotate" => match rest {
            [id, note @ ..] if !note.is_empty() => annotate(id, &note.join(" ")).await,
            _ => Err(usage(
                "Usage: hora annotate <incident-id|last> <note>\n\
                 An empty note (\"\") clears the annotation.",
            )),
        },
        "event" => event(rest).await,
        "timeline" => timeline(rest).await,
        "peers" => peers(rest).await,
        "postmortem" => {
            let id = rest
                .first()
                .ok_or_else(|| usage("Usage: hora postmortem <incident-id|last>"))?;
            postmortem(id).await
        }
        "silence" => silence(rest).await,
        "digest" => digest_preview().await,
        "report" => report(rest.first().map(String::as_str)).await,
        "doctor" => doctor().await,
        "tune" => tune(rest).await,
        "probe" => probe(rest).await,
        "announce" => announce(rest).await,
        "top" => top::run(rest).await.map_err(CliError::from),
        "--version" | "-V" => {
            println!("hora {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--help" | "-h" => {
            print_help();
            Ok(())
        }
        unknown => Err(usage(format!(
            "Unknown command: {unknown}\nRun 'hora --help' for usage information."
        ))),
    }
}

/// An optional `[limit]` argument (default 20, at least 1); `usage_line` is
/// the error when it is not a number.
fn parse_limit(raw: Option<&String>, usage_line: &str) -> Result<i64, CliError> {
    raw.map_or(Ok(20), |raw| raw.parse::<i64>())
        .map(|limit| limit.max(1))
        .map_err(|_| usage(usage_line))
}

/// A `--days N` value: a positive whole number of days.
fn parse_days(raw: Option<&String>) -> Result<i64, CliError> {
    raw.and_then(|raw| raw.parse::<i64>().ok())
        .filter(|&days| days > 0)
        .ok_or_else(|| usage("--days must be a positive number of days"))
}

/// The configured monitor `id`, or a usage error listing the configured ids:
/// a typo'd id fails loudly rather than testing, tuning or silencing nothing.
fn find_monitor<'c>(
    config: &'c hora_core::config::Config,
    id: &str,
) -> Result<&'c hora_core::config::Monitor, CliError> {
    config
        .find_monitor(id)
        .ok_or_else(|| unknown_monitor(config, id))
}

/// The usage error for an unknown monitor id, listing the configured ones.
fn unknown_monitor(config: &hora_core::config::Config, id: &str) -> CliError {
    let mut message = format!("Unknown monitor {id:?}. Configured ids:");
    for monitor in &config.monitors {
        let _ = write!(message, "\n  {}", monitor.id);
    }
    usage(message)
}

/// Resolve an incident argument: a numeric id, or `last` for the most recent
/// incident.
async fn resolve_incident_id(pool: &hora_core::db::SqlitePool, arg: &str) -> Result<i64, CliError> {
    if arg == "last" {
        return hora_core::db::latest_incident_id(pool)
            .await?
            .ok_or_else(|| CliError::Failed("No incidents recorded yet.".to_owned()));
    }
    arg.parse().map_err(|_| {
        usage(format!(
            "Invalid incident id {arg:?} (a number, or 'last')."
        ))
    })
}

/// The error for an incident id that matches no row.
fn no_such_incident(id: i64) -> CliError {
    CliError::Failed(format!(
        "No incident #{id}. 'hora incidents' lists the recent ones."
    ))
}

/// `hora import kuma <backup.json>`: convert an Uptime Kuma backup to Hora
/// TOML on stdout.
fn import_kuma(args: &[String]) -> Result<(), CliError> {
    let [kind, json_path, ..] = args else {
        return Err(usage("Usage: hora import kuma <backup.json>"));
    };
    if kind != "kuma" {
        return Err(usage("Usage: hora import kuma <backup.json>"));
    }
    let json_str =
        std::fs::read_to_string(json_path).with_context(|| format!("reading {json_path}"))?;
    let toml_out = hora_core::import::convert_kuma_to_hora(&json_str)?;
    println!("{toml_out}");
    Ok(())
}

/// `hora check`: validate the config and exit non-zero on error - meant for
/// CI and pre-deploy hooks.
fn check_config() -> Result<(), CliError> {
    let config_path = config::path();
    match config::load_from(&config_path) {
        Ok(_) => {
            println!("{} is valid.", config_path.display());
            Ok(())
        }
        Err(err) => Err(CliError::Failed(format!("Configuration error: {err:#}"))),
    }
}

/// Send a test `Down` then `Recovered` through the real notification chain, so
/// an operator verifies delivery *before* the first real incident instead of
/// during it. Without an id every configured channel is exercised; with one,
/// the monitor's `notify` routing applies - testing exactly what would fire.
/// Failures surface as the notifiers' own per-channel warnings *and* as a
/// non-zero exit, so a CI pipeline can gate on the notification chain.
async fn test_alert(monitor_id: Option<&str>) -> Result<(), CliError> {
    let config_path = config::path();
    let config = config::load_from(&config_path).context("loading configuration")?;

    let (name, notify) = match monitor_id {
        None => ("Hora test".to_owned(), None),
        Some(id) => {
            let monitor = find_monitor(&config, id)?;
            (monitor.name.clone(), monitor.notify.clone())
        }
    };

    let client = hora_core::http::client(None).context("building HTTP client")?;
    let dispatcher = hora_core::notifications::build(&config, &client, None);
    let targeted: Vec<&str> = dispatcher
        .names()
        .filter(|channel| {
            notify
                .as_ref()
                .is_none_or(|only| only.iter().any(|name| name == channel))
        })
        .collect();
    if targeted.is_empty() {
        return Err(CliError::Failed(
            "No notification channel to test (none configured, or none routed).".to_owned(),
        ));
    }

    println!(
        "Sending a test alert (down + recovered) as {name:?} to: {}",
        targeted.join(", ")
    );
    let event = hora_core::notifications::Event::Down {
        monitor: &name,
        error: Some("test alert sent by `hora test-alert` - not a real incident"),
        cause: None,
        impacted: &[],
        vantage: None,
        event: None,
    };
    let mut failed = dispatcher.dispatch(event, notify.as_deref()).await;
    let failed_recovery = dispatcher
        .dispatch(
            hora_core::notifications::Event::Recovered { monitor: &name },
            notify.as_deref(),
        )
        .await;
    // One verdict per channel: failing either delivery counts as failed.
    for channel in failed_recovery {
        if !failed.contains(&channel) {
            failed.push(channel);
        }
    }
    if failed.is_empty() {
        println!("Done. Every channel accepted both notifications.");
        Ok(())
    } else {
        Err(CliError::Failed(format!(
            "Delivery failed on: {} (the warnings above say why).",
            failed.join(", ")
        )))
    }
}

fn print_help() {
    println!("Hora - a tiny self-hosted uptime monitor");
    println!();
    println!("Usage: hora [COMMAND]");
    println!();
    println!("Commands:");
    println!("  import kuma <file>  Convert an Uptime Kuma backup JSON to Hora TOML (stdout)");
    println!("  check               Validate the configuration and exit");
    println!("  doctor              Diagnose the runtime environment (IPv6, ICMP socket,");
    println!("                      DNS resolver, listen port, database)");
    println!("  tune [id] [--days N]  Replay stored history against other fail_threshold /");
    println!("                      degraded_over_ms settings and recommend per monitor");
    println!("  probe <id|target> [--confirm] [--kind http|tcp|icmp|dns]");
    println!("                      One-shot ad-hoc probe; --confirm asks the peers their verdict");
    println!("  test-alert [id]     Send a test down + recovered through the configured");
    println!("                      channels (all of them, or the routed ones of monitor [id])");
    println!("  silence <ids> <for> [reason]  Mute alerts for monitors (comma-separated ids");
    println!("                      or 'all') for a duration like 10m or 1h30m (max 7d)");
    println!("  silence list        Show the active silences");
    println!("  announce <title> [body] [--severity s] [--until 4h|18:00]");
    println!("                      Pin a public banner on the status page");
    println!("  announce list / clear  Show or remove the pinned announcements");
    println!("                      ('hora announce -- clear ...' pins a title starting");
    println!("                      with list or clear)");
    println!("  silence clear       Remove every silence");
    println!("  top [--url U] [--token T] [--interval S]");
    println!("                      Live terminal dashboard over the JSON API");
    println!("  digest              Print the weekly digest (a dry run of [digest])");
    println!("  report [YYYY-MM]    Print the monthly SLA report (default: last month;");
    println!("                      the printable page is /report/YYYY-MM)");
    println!("  incidents [limit]   List recent incidents with their ids");
    println!("  annotate <id> <note>  Attach a note to an incident ('last' targets the");
    println!("                      most recent one; an empty note clears it)");
    println!("  event <title...>    Record an event marker (\"deploy api v2.3\"): shown on");
    println!("                      the charts and correlated into incidents");
    println!("  event list [limit]  List the recent event markers ('hora event -- list ...'");
    println!("                      records a title that starts with the word list)");
    println!("  postmortem <id|last>  Print an incident's auto-generated markdown");
    println!("                      post-mortem (the web twin is /incident/{{id}})");
    println!("  timeline [--days N] Unified chronology: downs/recoveries, events,");
    println!("                      pushed alerts, announcements, silences (default 7d)");
    println!("  peers diff          Compare this node's monitors with each peer's (the");
    println!("                      alignment multi-vantage confirmation relies on)");
    println!("  backup <dest.db>    Snapshot the database with VACUUM INTO");
    println!("  --version, -V       Show the version");
    println!("  --help, -h          Show this help message");
}

/// Open the daemon's database for a CLI subcommand. Refuses to *create* one: a
/// missing file means the config points somewhere the daemon never wrote (a
/// different working directory, usually), and silently creating an empty
/// database there would only hide the mistake.
async fn open_database() -> anyhow::Result<(hora_core::config::Config, hora_core::db::SqlitePool)> {
    let config_path = config::path();
    let config = config::load_from(&config_path).context("loading configuration")?;
    let path = &config.server.database_path;
    if path != ":memory:" && !path.starts_with("file:") && !std::path::Path::new(path).exists() {
        anyhow::bail!(
            "database {path} not found - run from the daemon's working directory, \
             or point HORA_CONFIG at its config"
        );
    }
    let pool = hora_core::db::connect(path)
        .await
        .context("opening database")?;
    Ok((config, pool))
}

/// Snapshot the database to `dest` via `VACUUM INTO`: consistent and compacted,
/// safe while the daemon runs. Meant for cron ("a one-statement answer to 'what
/// if I lose a year of history?'").
async fn backup(dest: &str) -> Result<(), CliError> {
    let config_path = config::path();
    let config = config::load_from(&config_path).context("loading configuration")?;
    let source = &config.server.database_path;
    hora_core::db::backup_into(source, dest).await?;
    let size = std::fs::metadata(dest).map_or(0, |meta| meta.len());
    println!("Backed up {source} to {dest} ({} KiB).", size / 1024);
    Ok(())
}

/// `hora announce`: pin (or list/clear) a public banner on the status page -
/// the communication side of incidents, written straight into the daemon's
/// database and live within the summary cache TTL (~5s). The HTTP twin is
/// `POST /api/announce`. A leading `--` ends the subcommand words, so a banner
/// titled "clear skies" is `hora announce -- clear skies`.
async fn announce(args: &[String]) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("--") if args.len() > 1 => pin_announcement(&args[1..]).await?,
        Some("list") => {
            let (_, pool) = open_database().await?;
            let now = chrono::Utc::now().timestamp();
            let pinned = hora_core::db::active_announcements(&pool, now).await?;
            if pinned.is_empty() {
                println!("No announcements pinned.");
            }
            for item in pinned {
                let until = item.until.map_or_else(
                    || "until cleared".to_owned(),
                    |ts| format!("until {}", fmt::utc(ts)),
                );
                println!("#{} [{}] {} ({until})", item.id, item.severity, item.title);
                if !item.body.is_empty() {
                    println!("      {}", item.body);
                }
            }
        }
        Some("clear") => {
            let (_, pool) = open_database().await?;
            let cleared =
                hora_core::db::clear_announcements(&pool, chrono::Utc::now().timestamp()).await?;
            println!("Cleared {cleared} announcement(s).");
        }
        Some(first) if first != "--" => pin_announcement(args).await?,
        _ => {
            return Err(usage(concat!(
                "Usage: hora announce <title> [body...] [--severity info|warning|critical|resolved] [--until 4h|18:00]\n",
                "       hora announce list\n",
                "       hora announce clear",
            )));
        }
    }
    Ok(())
}

async fn pin_announcement(args: &[String]) -> anyhow::Result<()> {
    let announcement = parse_announce_args(args, chrono::Utc::now().timestamp())?;
    let (_, pool) = open_database().await?;
    announcement.pin(&pool).await?;
    let expiry = announcement.until.map_or_else(
        || "until `hora announce clear`".to_owned(),
        |ts| format!("until {}", fmt::utc(ts)),
    );
    println!(
        "Pinned [{}] {:?} ({expiry}).",
        announcement.severity, announcement.title
    );
    Ok(())
}

/// Split `hora announce` arguments: `--severity`/`--until` flags anywhere
/// (see [`announce::split_flags`]), the first free word is the title, the rest
/// joins into the body.
fn parse_announce_args(args: &[String], now: i64) -> anyhow::Result<Announcement> {
    let flags = announce::split_flags(args.iter().map(String::as_str))?;
    let Some((title, body)) = flags.words.split_first() else {
        anyhow::bail!("announce needs a title");
    };
    let until = flags.until.and_then(|raw| announce::parse_until(raw, now));
    Ok(Announcement::new(
        title,
        &body.join(" "),
        flags.severity.unwrap_or_default(),
        until,
    )?)
}

/// Print the digest exactly as the `[digest]` task would send it - a dry run
/// to check the wording (and the data) without notifying anyone.
async fn digest_preview() -> Result<(), CliError> {
    let (config, pool) = open_database().await?;
    let now = chrono::Utc::now().timestamp();
    let (period, summary) = hora_core::digest::build_summary(&pool, &config, now).await?;
    println!("Hora digest ({period})");
    println!("{summary}");
    Ok(())
}

/// Diagnose the runtime environment against what the config needs: `hora
/// check` says the config is sound, `hora doctor` says the *host* can honour
/// it. Exits non-zero when a needed capability is missing.
async fn doctor() -> Result<(), CliError> {
    let config_path = config::path();
    let config = config::load_from(&config_path).context("loading configuration")?;
    println!(
        "hora doctor - {} ({} monitors)",
        config_path.display(),
        config.monitors.len()
    );
    println!();

    let findings = hora_core::doctor::run(&config).await;
    let mut failed = false;
    for finding in &findings {
        failed = failed || finding.status == hora_core::doctor::Status::Fail;
        println!(
            "  {:<10} {:<5} {}",
            finding.name,
            finding.status.label(),
            finding.detail
        );
    }
    if failed {
        println!();
        return Err(CliError::Failed(
            "Some capabilities the configuration needs are missing.".to_owned(),
        ));
    }
    Ok(())
}

/// Print the monthly SLA report as text - the terminal twin of the printable
/// `/report/{month}` page. Defaults to last month: "here is your May report".
async fn report(month: Option<&str>) -> Result<(), CliError> {
    let (config, pool) = open_database().await?;
    let month = month.map_or_else(
        || hora_core::report::previous_month(chrono::Utc::now().timestamp()),
        str::to_owned,
    );
    let report = hora_core::report::build(&pool, &config, &month)
        .await
        .map_err(|err| CliError::Failed(format!("{err:#}")))?;

    println!("SLA report - {} ({})", report.label, config.page.title);
    let mut current_group: Option<&str> = None;
    for row in &report.rows {
        let group = row.group.as_deref().unwrap_or("");
        if current_group != Some(group) {
            current_group = Some(group);
            println!();
            println!("{}", if group.is_empty() { "Monitors" } else { group });
        }
        let uptime = row
            .uptime_bp
            .map_or_else(|| "no data".to_owned(), fmt::pct_bp);
        let mut line = format!("  {}: {uptime}", row.name);
        if row.incidents > 0 {
            let plural = if row.incidents > 1 { "s" } else { "" };
            let _ = write!(
                line,
                ", {} incident{plural}, {} down",
                row.incidents,
                fmt::duration(row.downtime_secs)
            );
        }
        if let Some(mttr) = row.mttr_secs {
            let _ = write!(line, ", MTTR {}", fmt::duration(mttr));
        }
        if let (Some(slo_bp), Some(met)) = (row.slo_bp, row.slo_met) {
            let _ = write!(
                line,
                ", SLO {} {}",
                fmt::pct_bp(i64::from(slo_bp)),
                if met { "met" } else { "MISSED" }
            );
        }
        if let (Some(consumed), Some(budget)) = (row.budget_consumed_minutes, row.budget_minutes) {
            let _ = write!(line, ", budget {consumed}m of {budget}m");
        }
        println!("{line}");
    }
    Ok(())
}

/// `hora tune [monitor_id] [--days N]`: replay the stored check history against
/// alternative anti-flap settings and print per-monitor recommendations. Pure
/// read-only analytics over data that already exists - it never probes, never
/// writes. With an id, it focuses one monitor; without, every monitor that has
/// history. `--days` narrows the lookback (default: the monitor's retention).
async fn tune(args: &[String]) -> Result<(), CliError> {
    let mut only: Option<&str> = None;
    let mut days: Option<i64> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--days" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| usage("Usage: hora tune [monitor_id] [--days N]"))?;
                days = Some(parse_days(Some(raw))?);
            }
            other if only.is_none() => only = Some(other),
            other => {
                return Err(usage(format!(
                    "Unexpected argument {other:?} (usage: hora tune [monitor_id] [--days N])"
                )));
            }
        }
    }

    let (config, pool) = open_database().await?;
    let selected: Vec<&hora_core::config::Monitor> = match only {
        Some(id) => vec![find_monitor(&config, id)?],
        None => config.monitors.iter().collect(),
    };

    let lookback = days.unwrap_or_else(|| {
        // The widest per-monitor retention bounds how far raw checks reach.
        selected
            .iter()
            .map(|monitor| i64::from(monitor.retention_days(config.alerts.default_retention_days)))
            .max()
            .unwrap_or(90)
    });
    println!(
        "hora tune - {} (replaying up to {})",
        config::path().display(),
        days_label(lookback)
    );

    let now = chrono::Utc::now().timestamp();
    let mut printed = 0_usize;
    for monitor in selected {
        let retention = i64::from(monitor.retention_days(config.alerts.default_retention_days));
        let window_days = days.map_or(retention, |d| d.min(retention));
        let since = now - window_days * hora_core::SECONDS_PER_DAY;
        let samples = hora_core::db::check_samples(&pool, &monitor.id, since).await?;

        let ctx = hora_core::tune::MonitorContext {
            id: &monitor.id,
            name: &monitor.name,
            group: monitor.group.as_deref(),
            kind: monitor.kind.as_str(),
            interval_secs: monitor.interval_secs,
            current_threshold: config.alerts.fail_threshold,
            current_degraded_over_ms: monitor.degraded_over_ms,
        };
        let tuning = hora_core::tune::analyze(&ctx, &samples);

        if tuning.checks == 0 {
            // Only nag about an empty window when one monitor was asked for;
            // when sweeping all, silently skip the ones with no history.
            if only.is_some() {
                println!();
                println!(
                    "{} has no checks in the last {}.",
                    tuning.name,
                    days_label(window_days)
                );
            }
            continue;
        }
        println!();
        print_tuning(&tuning);
        printed += 1;
    }
    if printed == 0 && only.is_none() {
        println!();
        println!("No check history yet - let the daemon run, then tune against real data.");
    }
    Ok(())
}

/// Render one monitor's tuning block (see [`hora_core::tune`] for the model).
fn print_tuning(t: &hora_core::tune::MonitorTuning) {
    println!("{}  ({}, every {}s)", t.name, t.kind, t.interval_secs);

    let window = t.window.map_or_else(String::new, |(first, last)| {
        format!("{} -> {}, ", fmt::utc(first), fmt::utc(last))
    });
    println!(
        "  {} checks, {window}{} down  [fail_threshold={}]",
        t.checks, t.down_checks, t.current_threshold
    );

    if t.runs.is_empty() {
        println!("  no failures in the window - nothing to tune for fail_threshold");
    } else {
        let lengths: Vec<String> = t.runs.iter().map(ToString::to_string).collect();
        println!("  failure runs: {}  ({})", t.runs.len(), lengths.join(", "));
        println!("  fail_threshold   alerts   detect after");
        for row in &t.table {
            let marker = if row.threshold == t.current_threshold {
                "*"
            } else {
                " "
            };
            let current = if row.threshold == t.current_threshold {
                "  (current)"
            } else {
                ""
            };
            println!(
                "    {marker} {:<2}            {:>4}    {:>8}{current}",
                row.threshold,
                row.alerts,
                fmt::duration(row.detect_after_secs),
            );
        }
        print_threshold_advice(t);
    }

    print_degraded_advice(t);

    println!(
        "  probe_retries: not replayable from history (only a probe's final attempt is stored); \
         {} single-check failure{} seen - fail_threshold absorbs those across ticks",
        t.single_check_failures,
        if t.single_check_failures == 1 {
            ""
        } else {
            "s"
        }
    );
}

/// The one-line `fail_threshold` recommendation under the replay table.
fn print_threshold_advice(t: &hora_core::tune::MonitorTuning) {
    let interval = i64::try_from(t.interval_secs).unwrap_or(0);
    let longest = t.advice.longest_run;

    // Every failure was an isolated single-check blip: this is the anti-flap
    // question, not the outage-detection one. A threshold above 1 filtering
    // these is the design working - never a reason to lower it.
    if longest == 1 {
        if t.current_threshold <= 1 {
            println!(
                "  -> every failure was a single-check blip; raise fail_threshold to 2 so a \
                 one-off never pages"
            );
        } else {
            println!(
                "  -> only single-check blips ({}) occurred; fail_threshold {} filtered them all \
                 - the anti-flap working as intended",
                t.single_check_failures, t.current_threshold
            );
        }
        return;
    }

    // A multi-check outage that the current threshold never confirmed: a real
    // misconfiguration, this monitor would have stayed silent through it.
    if t.never_alerts() {
        println!(
            "  ! fail_threshold {} never fires here: the longest outage was {longest} checks - \
             lower it to {longest} or less to catch real outages",
            t.current_threshold
        );
        return;
    }

    let Some(rec) = t.advice.recommended else {
        println!(
            "  -> no clear flap/outage split in the runs above - the table shows the trade-off; \
             current fail_threshold {} looks reasonable",
            t.current_threshold
        );
        return;
    };
    let flap_max = t.advice.flap_max.unwrap_or(0);
    let rec_alerts = t
        .table
        .iter()
        .find(|row| row.threshold == rec)
        .map_or(0, |row| row.alerts);

    match rec.cmp(&t.current_threshold) {
        std::cmp::Ordering::Equal => println!(
            "  -> fail_threshold {rec} looks right: flaps (runs <= {flap_max}) filtered, \
             longest outage ({} checks) still caught",
            t.advice.longest_run
        ),
        std::cmp::Ordering::Greater => {
            let delay = i64::from(rec - t.current_threshold).saturating_mul(interval);
            println!(
                "  -> raise fail_threshold to {rec}: {} alerts instead of {} over the window \
                 (drops flaps <= {flap_max}), same real outages, +{} to detect",
                rec_alerts,
                t.current_alerts,
                fmt::duration(delay)
            );
        }
        std::cmp::Ordering::Less => {
            let saved = i64::from(t.current_threshold - rec).saturating_mul(interval);
            println!(
                "  -> fail_threshold {rec} would catch the same outages {} sooner; \
                 the current {} only adds delay (no extra flaps above {flap_max} to filter)",
                fmt::duration(saved),
                t.current_threshold
            );
        }
    }
}

/// The one-line `degraded_over_ms` recommendation from the latency spread.
fn print_degraded_advice(t: &hora_core::tune::MonitorTuning) {
    let Some(stats) = &t.latency else {
        return;
    };
    println!(
        "  latency, up checks: p50 {}ms  p95 {}ms  p99 {}ms  max {}ms{}",
        stats.p50,
        stats.p95,
        stats.p99,
        stats.max,
        t.current_degraded_over_ms
            .map_or_else(String::new, |ms| format!("  [degraded_over_ms={ms}]"))
    );
    let Some(rec) = t.recommended_degraded_over_ms else {
        return;
    };
    match (t.current_degraded_over_ms, t.currently_degraded) {
        (Some(current), Some(flagged)) => {
            let pct = percent(flagged, stats.count);
            if current < stats.p95 {
                println!(
                    "    {current}ms flags {flagged} of {} up checks ({pct}%) - that is normal \
                     traffic; recommend degraded_over_ms {rec} (~p99, ~1%)",
                    stats.count
                );
            } else {
                println!(
                    "    {current}ms flags {flagged} of {} up checks ({pct}%); ~p99 is {rec}ms",
                    stats.count
                );
            }
        }
        _ => println!(
            "    no degraded_over_ms set - recommend {rec} (~p99) to flag genuine slowness"
        ),
    }
}

/// `"1 day"` / `"30 days"` - the only place the lookback is pluralised.
fn days_label(days: i64) -> String {
    format!("{days} day{}", if days == 1 { "" } else { "s" })
}

/// Percentage of `part` in `whole`, one decimal place; `0.0` for an empty whole.
/// Integer math (tenths of a percent, rounded) to stay exact and lint-clean.
fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "0.0".to_owned();
    }
    let tenths = (part * 1000 + whole / 2) / whole;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// `hora probe <monitor-id|target> [--confirm] [--kind http|tcp|icmp|dns]`: a
/// one-shot ad-hoc probe from the terminal with full monitor semantics (status,
/// latency, HTTP/DNS assertions, TLS expiry). A bare argument matching a
/// configured monitor id is probed with that monitor's exact config; anything
/// else is an ad-hoc target whose kind is inferred (or forced with `--kind`).
/// `--confirm` asks the configured peers to probe the same target - the
/// distributed "down for everyone or just me?" in one command. Exits non-zero
/// when the target is down, so it doubles as a scriptable health check.
async fn probe(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_probe_args(args)?;
    let config = config::load_from(&config::path()).context("loading configuration")?;

    // A bare id (no --kind) reuses that monitor's full config - assertions,
    // cert pin, dns expectation and all. --kind always means "ad-hoc target".
    let configured = parsed
        .kind_override
        .is_none()
        .then(|| config.find_monitor(parsed.target))
        .flatten();
    let mut monitor = if let Some(found) = configured {
        found.clone()
    } else {
        let (kind, target) = match parsed.kind_override {
            Some(kind) => (kind, hora_core::config::probe_target(kind, parsed.target)),
            None => hora_core::config::infer_probe(parsed.target),
        };
        hora_core::config::Monitor::ad_hoc(kind, target)
    };
    // Push and exec monitors are evaluated from heartbeats / an external
    // command, not a live network probe - reject them whichever way the monitor
    // was resolved (a configured id, or an inferred kind).
    if matches!(
        monitor.kind,
        hora_core::config::Kind::Push | hora_core::config::Kind::Exec
    ) {
        return Err(CliError::Failed(format!(
            "hora probe runs a live network check, but {:?} is a {} monitor (no active probe)",
            monitor.id,
            monitor.kind.as_str()
        )));
    }
    // Force confirmation on regardless of the monitor's own setting: the flag
    // is the explicit ask. confirm_with_peers is read by confirm::enabled only.
    if parsed.confirm {
        monitor.confirm_with_peers = Some(true);
    }

    let header = if configured.is_some() {
        format!(
            "{} ({}, {})",
            monitor.name,
            monitor.kind.as_str(),
            monitor.target
        )
    } else {
        format!("{} {}", monitor.kind.as_str(), monitor.target)
    };
    println!("hora probe - {header}");
    println!();

    let client = hora_core::http::probe_client(monitor.proxy.as_deref())
        .context("building the probe HTTP client")?;
    let outcome = hora_core::probe::run(&client, &monitor).await;
    print_probe_report(&monitor, &outcome).await;

    if parsed.confirm {
        confirm_probe(&config, &monitor, &outcome).await?;
    }

    // Down exits non-zero so `hora probe url && deploy` works as a gate;
    // degraded is still up.
    if !outcome.up {
        return Err(CliError::Silent);
    }
    Ok(())
}

/// A parsed `hora probe` invocation. Borrows the target straight from `args`.
#[derive(Debug)]
struct ProbeArgs<'a> {
    target: &'a str,
    confirm: bool,
    kind_override: Option<hora_core::config::Kind>,
}

/// Parse `hora probe`'s arguments: a usage error on anything malformed (a
/// missing target, an unknown flag, a bad `--kind`, extra args).
fn parse_probe_args(args: &[String]) -> Result<ProbeArgs<'_>, CliError> {
    let mut target: Option<&str> = None;
    let mut confirm = false;
    let mut kind_override: Option<hora_core::config::Kind> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--confirm" => confirm = true,
            "--kind" => {
                let raw = iter.next().map_or("", String::as_str);
                kind_override =
                    Some(parse_probe_kind(raw).ok_or_else(|| {
                        usage("--kind must be one of: http, tcp, icmp (ping), dns")
                    })?);
            }
            flag if flag.starts_with('-') => {
                return Err(usage(format!(
                    "Unknown option {flag:?} (usage: hora probe <id|target> [--confirm] [--kind K])"
                )));
            }
            value if target.is_none() => target = Some(value),
            extra => {
                return Err(usage(format!(
                    "Unexpected argument {extra:?} - hora probe takes a single target"
                )));
            }
        }
    }
    let target = target.ok_or_else(|| {
        usage("Usage: hora probe <monitor-id|target> [--confirm] [--kind http|tcp|icmp|dns]")
    })?;
    Ok(ProbeArgs {
        target,
        confirm,
        kind_override,
    })
}

/// Print the probe outcome, then the TLS expiry for https targets - the latter
/// fail-open like the watcher: a handshake failure annotates, never changes the
/// verdict.
async fn print_probe_report(
    monitor: &hora_core::config::Monitor,
    outcome: &hora_core::probe::Outcome,
) {
    let status = if !outcome.up {
        "DOWN"
    } else if outcome.degraded {
        "DEGRADED (up but slow)"
    } else {
        "UP"
    };
    println!("  status    {status}");
    println!(
        "  latency   {}",
        outcome
            .latency_ms
            .map_or_else(|| "-".to_owned(), |ms| format!("{ms} ms"))
    );
    if let Some(code) = outcome.status_code {
        println!("  code      {code}");
    }
    if let Some(error) = &outcome.error {
        println!("  error     {error}");
    }
    if let Some(first_line) = outcome
        .snapshot
        .as_deref()
        .and_then(|snapshot| snapshot.lines().next())
    {
        println!("  answered  {first_line}");
    }

    // Only read the certificate when the probe actually reached the server (up,
    // or a response like a 5xx where TLS still answered). A transport failure
    // means the cert handshake would just hit the same timeout twice.
    let reached_server = outcome.up || outcome.status_code.is_some();
    let https =
        monitor.kind == hora_core::config::Kind::Http && monitor.target.starts_with("https://");
    let starttls = hora_core::cert::Starttls::for_monitor(monitor);
    if reached_server && (https || starttls.is_some()) {
        // STARTTLS tcp monitors read their certificate exactly like the
        // watcher: negotiate in plaintext, then handshake.
        let cert = match hora_core::cert::monitor_endpoint(monitor) {
            Some((host, port)) => {
                hora_core::cert::inspect_endpoint(
                    &host,
                    port,
                    starttls,
                    monitor.ehlo_name.as_deref(),
                    monitor.timeout(),
                )
                .await
            }
            None => Err(anyhow::anyhow!("cannot determine host:port")),
        };
        match cert {
            Ok(cert) => println!(
                "  cert      {} (expires {})",
                cert_days_phrase(cert.days_left),
                format_date(cert.not_after)
            ),
            Err(err) => println!("  cert      could not read certificate: {err}"),
        }
    }
}

/// Ask the configured peers for their verdict on the target and print the
/// multi-vantage summary from this node's perspective: a local down asks "down
/// for everyone or just me?", a local up asks "is it up from elsewhere too?".
/// Fail-open: peers being absent or unreachable never errors.
async fn confirm_probe(
    config: &hora_core::config::Config,
    monitor: &hora_core::config::Monitor,
    outcome: &hora_core::probe::Outcome,
) -> anyhow::Result<()> {
    let plain = hora_core::http::client(None).context("building the confirm HTTP client")?;
    match hora_core::confirm::confirm_verdict(&plain, config, monitor, outcome.up).await {
        Some(verdict) => println!("  vantage   {verdict}"),
        None => {
            println!(
                "  vantage   no peers to ask - needs [health].id and [[peers]] with a ping_url"
            );
        }
    }
    Ok(())
}

/// Parse a `--kind` value into a probeable monitor kind (push/exec excluded).
fn parse_probe_kind(raw: &str) -> Option<hora_core::config::Kind> {
    use hora_core::config::Kind;
    match raw.to_ascii_lowercase().as_str() {
        "http" => Some(Kind::Http),
        "tcp" => Some(Kind::Tcp),
        "icmp" | "ping" => Some(Kind::Icmp),
        "dns" => Some(Kind::Dns),
        _ => None,
    }
}

/// Human phrase for certificate days-left, handling the already-expired case.
fn cert_days_phrase(days_left: i64) -> String {
    if days_left < 0 {
        format!("EXPIRED {} days ago", -days_left)
    } else {
        format!("{days_left} days left")
    }
}

/// Format a unix timestamp as a `YYYY-MM-DD` date (UTC).
fn format_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0).map_or_else(
        || timestamp.to_string(),
        |dt| dt.format("%Y-%m-%d").to_string(),
    )
}

/// `hora event <title...>` / `hora event list [N]`: record (or list) an event
/// marker - "deploy api v2.3" - the same gesture as a silence, written
/// straight into the daemon's database. Markers overlay the latency
/// sparklines, list on /history, and a down confirming within the hour is
/// annotated "recent change: ...". The HTTP twin is `POST /api/event`.
async fn event(args: &[String]) -> Result<(), CliError> {
    match parse_event_args(args)? {
        EventCommand::List(limit) => {
            let (_, pool) = open_database().await?;
            let events = hora_core::db::recent_events(&pool, limit).await?;
            if events.is_empty() {
                println!("No events recorded.");
            }
            for event in events {
                println!("{}  {}", fmt::utc(event.created_at), event.title);
            }
        }
        EventCommand::Record(title) => {
            let (_, pool) = open_database().await?;
            hora_core::db::insert_event(&pool, &title).await?;
            println!("Recorded event: {title}");
        }
    }
    Ok(())
}

/// What `hora event` was asked to do.
#[derive(Debug, PartialEq, Eq)]
enum EventCommand {
    /// `list [limit]`.
    List(i64),
    /// Record a marker with this title (trimmed, at most 200 chars).
    Record(String),
}

/// Parse `hora event` arguments. `list` is the subcommand only when followed
/// by nothing or a number: `hora event list deploy` is a usage error that
/// points at `--` rather than guessing - a leading `--` ends the subcommand
/// words, so `hora event -- list deploy` records "list deploy".
fn parse_event_args(args: &[String]) -> Result<EventCommand, CliError> {
    let title_words = match args {
        [] => {
            return Err(usage(
                "Usage: hora event <title...>\n       hora event list [limit]",
            ));
        }
        [first, rest @ ..] if first == "list" => {
            return match rest.first().map(|raw| raw.parse::<i64>()) {
                None => Ok(EventCommand::List(20)),
                Some(Ok(limit)) => Ok(EventCommand::List(limit.max(1))),
                Some(Err(_)) => Err(usage(format!(
                    "Usage: hora event list [limit]\n\
                     To record an event whose title starts with \"list\": hora event -- {}",
                    args.join(" ")
                ))),
            };
        }
        [first, rest @ ..] if first == "--" => rest,
        _ => args,
    };
    let title = hora_core::bounded(&title_words.join(" "), MAX_EVENT_TITLE_CHARS);
    if title.is_empty() {
        return Err(usage("Usage: hora event <title...>"));
    }
    Ok(EventCommand::Record(title))
}

/// `hora peers diff`: compare this node's probeable monitors (kind + target)
/// with each peer's, over the authenticated `/api/peer/monitors` exchange.
/// `confirm_with_peers` only works on monitors both nodes know, and nothing
/// else verifies that alignment - this does, and exits non-zero on any drift
/// or unreachable peer so it can gate a config deploy.
async fn peers(args: &[String]) -> Result<(), CliError> {
    if args.first().map(String::as_str) != Some("diff") {
        return Err(usage("Usage: hora peers diff"));
    }
    let config = config::load_from(&config::path()).context("loading configuration")?;
    let Some(from) = config.health.as_ref().map(|health| health.id.clone()) else {
        return Err(CliError::Failed(
            "hora peers diff needs [health].id (the identity the peers authenticate).".to_owned(),
        ));
    };
    // Each askable peer paired with the URL that made it askable.
    let askable: Vec<(&hora_core::config::Peer, String)> = config
        .peers
        .iter()
        .filter_map(|peer| peer.monitors_url().map(|url| (peer, url)))
        .collect();
    if askable.is_empty() {
        return Err(CliError::Failed(
            "No askable peers: [[peers]] entries need a ping_url to derive the API origin."
                .to_owned(),
        ));
    }

    let local: std::collections::BTreeSet<(String, String)> = config
        .monitors
        .iter()
        .filter(|monitor| {
            !matches!(
                monitor.kind,
                hora_core::config::Kind::Push | hora_core::config::Kind::Exec
            )
        })
        .map(|monitor| (monitor.kind.as_str().to_owned(), monitor.target.clone()))
        .collect();

    let client = hora_core::http::client(None).context("building HTTP client")?;
    let mut drift = false;
    println!("hora peers diff - {} local probeable monitors", local.len());
    for (peer, url) in askable {
        let answer = hora_core::vantage::fetch_peer_monitors(
            &client,
            &url,
            &from,
            peer.ping_token.as_ref().map(AsRef::as_ref),
        )
        .await;
        println!();
        drift |= print_peer_diff(&peer.name, &local, answer.as_ref());
    }
    if drift {
        println!();
        println!("Configs have drifted - multi-vantage confirmation only covers shared monitors.");
        return Err(CliError::Silent);
    }
    Ok(())
}

/// Print one peer's diff against the local monitor set; `true` means drift
/// (or an unreachable peer - a mesh you cannot ask is a mesh out of sync).
fn print_peer_diff(
    peer_name: &str,
    local: &std::collections::BTreeSet<(String, String)>,
    answer: Option<&hora_core::confirm::PeerMonitors>,
) -> bool {
    let Some(answer) = answer else {
        println!("{peer_name}: UNREACHABLE (or refused the exchange)");
        return true;
    };
    let remote: std::collections::BTreeSet<(String, String)> = answer
        .monitors
        .iter()
        .map(|monitor| (monitor.kind.as_str().to_owned(), monitor.target.clone()))
        .collect();
    let missing: Vec<_> = local.difference(&remote).collect();
    let extra: Vec<_> = remote.difference(local).collect();
    if missing.is_empty() && extra.is_empty() {
        println!("{peer_name}: in sync ({} monitors)", remote.len());
        return false;
    }
    println!("{peer_name}: DRIFT");
    for (kind, target) in missing {
        println!("  missing there: {kind} {target}");
    }
    for (kind, target) in extra {
        println!("  only there:    {kind} {target}");
    }
    true
}

/// `hora timeline [--days N]`: the unified chronology - down/recovered
/// transitions, operator events, pushed alerts, announcements, silences -
/// merged newest-first. "What happened this week?" in one command.
async fn timeline(args: &[String]) -> Result<(), CliError> {
    let mut days: i64 = 7;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--days" => days = parse_days(iter.next())?,
            other => {
                return Err(usage(format!(
                    "Unexpected argument {other:?} (usage: hora timeline [--days N])"
                )));
            }
        }
    }

    let (config, pool) = open_database().await?;
    let since = chrono::Utc::now().timestamp() - days * hora_core::SECONDS_PER_DAY;
    let sources = hora_core::timeline::fetch(&pool, since, 500).await?;
    let names: std::collections::HashMap<String, String> = config
        .monitors
        .iter()
        .map(|monitor| (monitor.id.clone(), monitor.name.clone()))
        .collect();
    let entries = hora_core::timeline::merge(&sources, &names, since, 200);

    println!("hora timeline - last {}", days_label(days));
    if entries.is_empty() {
        println!();
        println!("Nothing recorded in the window.");
        return Ok(());
    }
    for entry in entries {
        let link = entry
            .incident_id
            .map(|id| format!("  (#{id})"))
            .unwrap_or_default();
        println!(
            "{}  {:<9}  {}{link}",
            fmt::utc(entry.at),
            entry.kind.as_str(),
            entry.title
        );
        if let Some(detail) = entry.detail {
            // Aligned under the title column (timestamp + gap + kind + gap).
            println!("{:>36}{detail}", "");
        }
    }
    Ok(())
}

/// `hora postmortem <id|last>`: print the auto-generated markdown post-mortem
/// of an incident - everything Hora already recorded about it, ready to paste
/// into a ticket. The web twin is `/incident/{id}`.
async fn postmortem(id_arg: &str) -> Result<(), CliError> {
    let (config, pool) = open_database().await?;
    let id = resolve_incident_id(&pool, id_arg).await?;
    let Some(incident) = hora_core::db::incident_by_id(&pool, id).await? else {
        return Err(no_such_incident(id));
    };
    let name = config.monitor_name(&incident.monitor_id);
    print!("{}", hora_core::postmortem::render(&incident, name));
    Ok(())
}

/// List recent incidents with their ids - the lookup companion of `annotate`.
async fn list_incidents(limit: i64) -> Result<(), CliError> {
    let (config, pool) = open_database().await?;
    let incidents = hora_core::db::recent_incidents(&pool, limit).await?;
    if incidents.is_empty() {
        println!("No incidents recorded.");
        return Ok(());
    }
    for incident in incidents {
        let name = config.monitor_name(&incident.monitor_id);
        let span = match incident.ended_at {
            Some(ended) => format!(
                "{} -> {} ({})",
                fmt::utc(incident.started_at),
                fmt::utc(ended),
                fmt::duration(incident.duration_s.unwrap_or(0))
            ),
            None => format!("{} -> ongoing", fmt::utc(incident.started_at)),
        };
        println!("#{}  {name}  {span}", incident.id);
        if let Some(error) = &incident.error {
            println!("      error: {error}");
        }
        // The full snapshot lives on /history; the status line is enough here.
        if let Some(first_line) = incident
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.lines().next())
        {
            println!("      answered: {first_line}");
        }
        if let Some(note) = &incident.note {
            println!("      note:  {note}");
        }
    }
    Ok(())
}

/// Attach (or clear, with an empty note) an annotation on an incident, shown
/// on /history and in the Atom feed. `last` targets the most recent incident.
async fn annotate(id_arg: &str, note: &str) -> Result<(), CliError> {
    let (_, pool) = open_database().await?;
    let id = resolve_incident_id(&pool, id_arg).await?;
    if !hora_core::db::set_incident_note(&pool, id, note).await? {
        return Err(no_such_incident(id));
    }
    if note.is_empty() {
        println!("Cleared the note on incident #{id}.");
    } else {
        println!("Annotated incident #{id}: {note}");
    }
    Ok(())
}

/// `hora silence <ids|all> <duration> [reason]` / `list` / `clear`: ad-hoc
/// alert muting (a deploy window) written straight into the daemon's database,
/// picked up on its next tick. The HTTP counterpart is `POST /api/silence`.
async fn silence(args: &[String]) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("list") => {
            let (_, pool) = open_database().await?;
            let now = chrono::Utc::now().timestamp();
            let silences = hora_core::db::active_silences(&pool, now).await?;
            if silences.is_empty() {
                println!("No active silences.");
            }
            for silence in silences {
                let target = if silence.monitor_id == "*" {
                    "all monitors"
                } else {
                    &silence.monitor_id
                };
                let reason = silence
                    .reason
                    .map(|reason| format!(" - {reason}"))
                    .unwrap_or_default();
                println!(
                    "{target}: until {} ({} left){reason}",
                    fmt::utc(silence.until),
                    fmt::duration(silence.until - now)
                );
            }
        }
        Some("clear") => {
            let (_, pool) = open_database().await?;
            let cleared =
                hora_core::db::clear_silences(&pool, chrono::Utc::now().timestamp()).await?;
            println!("Cleared {cleared} active silence(s).");
        }
        Some(ids) if args.len() >= 2 => {
            let (config, pool) = open_database().await?;
            let reason = (args.len() > 2).then(|| args[2..].join(" "));
            let silenced =
                match hora_core::silence::apply(&pool, &config, ids, &args[1], reason.as_deref())
                    .await
                {
                    Ok(silenced) => silenced,
                    Err(SilenceError::InvalidDuration) => {
                        return Err(usage(format!(
                            "Invalid duration {:?} (use e.g. 10m, 1h30m; max 7d).",
                            args[1]
                        )));
                    }
                    Err(SilenceError::UnknownId(id)) => return Err(unknown_monitor(&config, &id)),
                    Err(SilenceError::NoIds) => return Err(usage(SILENCE_USAGE)),
                    Err(err @ SilenceError::Database(_)) => {
                        return Err(CliError::Other(err.into()));
                    }
                };
            let until = silenced.until;
            let target = if silenced.ids == ["*"] {
                "all monitors".to_owned()
            } else {
                silenced.ids.join(", ")
            };
            println!("Silenced {target} until {}.", fmt::utc(until));
        }
        _ => return Err(usage(SILENCE_USAGE)),
    }
    Ok(())
}

/// What `hora silence` answers a malformed invocation with.
const SILENCE_USAGE: &str = concat!(
    "Usage: hora silence <ids|all> <duration> [reason]\n",
    "       hora silence list\n",
    "       hora silence clear",
);

/// Run the monitor: load config, open the database, start the supervisor and
/// background tasks, and serve the status page until a shutdown signal.
async fn serve() -> anyhow::Result<()> {
    let config_path = config::path();
    let initial = config::load_from(&config_path).context("loading configuration")?;
    let pool = hora_core::db::connect(&initial.server.database_path)
        .await
        .context("opening database")?;
    // The notifier client (no proxy); per-monitor probe clients are built by the
    // supervisor so each can carry its own proxy.
    let client = hora_core::http::client(None).context("building HTTP client")?;

    // A shutdown signal lets the background tasks stop cleanly (finishing their
    // current iteration) instead of being aborted when the runtime drops.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // The scheduler's liveness beacon: each monitor tick bumps it, and the
    // dead-man heartbeat and /healthz read it to tell a live scheduler from a
    // wedged one. Shared with the supervisor (writers) and the web layer (reader).
    let last_tick = Arc::new(AtomicU64::new(0));

    // The supervisor owns the live config + notification channels and reconciles
    // monitor tasks on reload; other components read through its handles.
    let handle = hora_core::supervisor::start(
        initial,
        config_path,
        pool.clone(),
        client.clone(),
        Arc::clone(&last_tick),
        shutdown_rx.clone(),
    );
    let cert_task = hora_core::cert::spawn_watcher(
        pool.clone(),
        handle.config.clone(),
        handle.notifier.clone(),
        client.clone(),
        shutdown_rx.clone(),
    );

    // Mutual surveillance: the outbound dead-man heartbeat. It self-gates on the
    // [health] section and reads it live, so it is always spawned (and activates if
    // [health] is added on reload). The inbound peer-watch tasks are owned and
    // hot-reloaded by the supervisor alongside the monitors.
    let heartbeat_task = hora_core::peer::spawn_heartbeat(
        handle.config.clone(),
        pool.clone(),
        client.clone(),
        Arc::clone(&last_tick),
        shutdown_rx.clone(),
    );

    // Per-vantage latency: poll the peers' /api/peer/monitors in the
    // background and share the snapshot with the web layer. Self-gating like
    // the heartbeat (no peers = a no-op per round).
    let vantage_map = hora_core::vantage::new_map();
    let vantage_task = hora_core::vantage::spawn_poller(
        handle.config.clone(),
        client,
        Arc::clone(&vantage_map),
        shutdown_rx.clone(),
    );

    let digest_task = hora_core::digest::spawn(
        pool.clone(),
        handle.config.clone(),
        handle.notifier.clone(),
        shutdown_rx.clone(),
    );

    let prune_task = hora_core::db::spawn_pruner(&pool, handle.config.clone(), shutdown_rx);

    let bind = handle.config.borrow().server.bind.clone();
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(
        "hora {} listening on http://{bind}",
        env!("CARGO_PKG_VERSION")
    );

    let state = hora_web::AppState::new(
        pool,
        handle.config.clone(),
        Arc::clone(&last_tick),
        handle.notifier.clone(),
    )
    .with_vantage(vantage_map);
    // Connect-info gives the rate limiter the peer socket IP: the client key
    // unless `server.client_ip_header` names a trusted proxy header (forwarded
    // headers are never read by default - a direct client could forge them).
    let app = hora_web::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("running HTTP server")?;

    // The HTTP server has drained; now stop the background tasks and wait briefly
    // for them to finish their current iteration before the runtime drops.
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        let _ = tokio::join!(
            handle.task,
            cert_task,
            prune_task,
            heartbeat_task,
            digest_task,
            vantage_task
        );
    })
    .await;
    Ok(())
}

fn init_tracing() {
    // Distinguish "unset" (silent default) from "set but invalid" (warn, so a
    // typo'd filter isn't silently ignored). Tracing isn't up yet, so use stderr.
    let filter = match std::env::var("HORA_LOG") {
        Ok(value) => EnvFilter::try_new(&value).unwrap_or_else(|err| {
            eprintln!("warning: invalid HORA_LOG {value:?} ({err}); using info");
            EnvFilter::new("info")
        }),
        Err(_) => EnvFilter::new("info"),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Resolve when the process receives a shutdown signal. Listens for Ctrl-C on
/// every platform and, on Unix, also `SIGTERM` - the signal `docker stop` and
/// most init systems send - so the server drains in-flight requests cleanly
/// instead of being killed after the grace period.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to listen for Ctrl-C: {err}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::error!("failed to listen for SIGTERM: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|&part| part.to_owned()).collect()
    }

    #[test]
    fn announce_args_split_flags_title_and_body() {
        let announcement = parse_announce_args(
            &strings(&[
                "Fiber",
                "cut,",
                "ETA",
                "6pm",
                "--severity",
                "warning",
                "--until",
                "4h",
            ]),
            1000,
        )
        .expect("parse");
        assert_eq!(announcement.title, "Fiber");
        assert_eq!(announcement.body, "cut, ETA 6pm");
        assert_eq!(announcement.severity, announce::Severity::Warning);
        assert_eq!(announcement.until, Some(1000 + 4 * 3600));

        assert!(parse_announce_args(&strings(&["--severity", "warning"]), 0).is_err());
        assert!(parse_announce_args(&strings(&["t", "--severity", "panic"]), 0).is_err());
        assert!(parse_announce_args(&strings(&["t", "--until", "nope"]), 0).is_err());
    }

    /// The message a usage error would print, or a panic for any other outcome.
    fn usage_message<T: std::fmt::Debug>(result: Result<T, CliError>) -> String {
        match result {
            Err(CliError::Usage(message)) => message,
            other => panic!("expected a usage error, got {other:?}"),
        }
    }

    fn two_monitor_config() -> hora_core::config::Config {
        hora_core::config::parse(
            r#"
            [page]
            [server]
            [[monitors]]
            id = "api"
            name = "Public API"
            target = "https://api.example.com"
            interval_secs = 60
            [[monitors]]
            id = "db"
            name = "Database"
            target = "db.internal:5432"
            kind = "tcp"
            interval_secs = 60
            "#,
        )
        .expect("config")
    }

    #[test]
    fn probe_args_return_usage_errors_instead_of_exiting() {
        let args = strings(&["example.com", "--confirm", "--kind", "ping"]);
        let parsed = parse_probe_args(&args).expect("parse");
        assert_eq!(parsed.target, "example.com");
        assert!(parsed.confirm);
        assert_eq!(parsed.kind_override, Some(hora_core::config::Kind::Icmp));

        assert!(usage_message(parse_probe_args(&[])).starts_with("Usage: hora probe"));
        assert!(usage_message(parse_probe_args(&strings(&["a", "b"]))).contains("single target"));
        assert!(usage_message(parse_probe_args(&strings(&["--nope"]))).contains("Unknown option"));
        assert!(
            usage_message(parse_probe_args(&strings(&["a", "--kind", "smtp"])))
                .starts_with("--kind must be")
        );
    }

    #[test]
    fn event_args_disambiguate_list_from_a_title() {
        assert_eq!(
            parse_event_args(&strings(&["list"])).expect("list"),
            EventCommand::List(20)
        );
        assert_eq!(
            parse_event_args(&strings(&["list", "0"])).expect("list 0"),
            EventCommand::List(1)
        );
        assert_eq!(
            parse_event_args(&strings(&["deploy", "api", "v2.3"])).expect("record"),
            EventCommand::Record("deploy api v2.3".to_owned())
        );
        // `list <words>` is ambiguous: refuse, and point at the `--` escape.
        let message = usage_message(parse_event_args(&strings(&["list", "deploy"])));
        assert!(message.contains("hora event -- list deploy"), "{message}");
        assert_eq!(
            parse_event_args(&strings(&["--", "list", "deploy"])).expect("escaped"),
            EventCommand::Record("list deploy".to_owned())
        );
        // Titles are capped like the API's; empty ones are refused.
        let long = "x".repeat(300);
        assert_eq!(
            parse_event_args(&strings(&[&long])).expect("long"),
            EventCommand::Record("x".repeat(MAX_EVENT_TITLE_CHARS))
        );
        assert!(usage_message(parse_event_args(&[])).starts_with("Usage: hora event"));
        usage_message(parse_event_args(&strings(&["--"])));
        usage_message(parse_event_args(&strings(&["  "])));
    }

    #[test]
    fn unknown_monitor_lists_the_configured_ids() {
        let config = two_monitor_config();
        assert_eq!(find_monitor(&config, "db").expect("db").name, "Database");
        assert_eq!(
            usage_message(find_monitor(&config, "dbb")),
            "Unknown monitor \"dbb\". Configured ids:\n  api\n  db"
        );
        assert_eq!(config.monitor_name("api"), "Public API");
        // A monitor removed since its incident was recorded keeps its id.
        assert_eq!(config.monitor_name("gone"), "gone");
    }

    #[test]
    fn limit_and_days_flags_validate() {
        assert_eq!(parse_limit(None, "u").expect("default"), 20);
        assert_eq!(
            parse_limit(Some(&"-5".to_owned()), "u").expect("clamped"),
            1
        );
        assert_eq!(usage_message(parse_limit(Some(&"x".to_owned()), "u")), "u");
        assert_eq!(parse_days(Some(&"3".to_owned())).expect("days"), 3);
        usage_message(parse_days(Some(&"0".to_owned())));
        usage_message(parse_days(None));
    }

    #[tokio::test]
    async fn incident_ids_resolve_last_or_a_number() {
        let pool = hora_core::db::connect(":memory:").await.expect("db");
        assert_eq!(resolve_incident_id(&pool, "42").await.expect("number"), 42);
        assert!(
            usage_message(resolve_incident_id(&pool, "4x2").await).contains("a number, or 'last'")
        );
        match resolve_incident_id(&pool, "last").await {
            Err(CliError::Failed(message)) => assert_eq!(message, "No incidents recorded yet."),
            other => panic!("expected a failure on an empty database, got {other:?}"),
        }
    }
}
