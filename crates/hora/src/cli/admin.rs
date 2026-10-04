//! Operating the instance: config check, test alert, backup, digest preview,
//! doctor and the Uptime Kuma import.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Context as _;
use hora_core::config;

use super::{CliError, find_monitor, open_database, usage};

/// `hora import kuma <backup.json>`: convert an Uptime Kuma backup to Hora
/// TOML on stdout.
pub(crate) fn import_kuma(args: &[String]) -> Result<(), CliError> {
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

/// `hora check [--strict]`: validate the config, print every warning on
/// stderr, and exit non-zero on error - or, with `--strict`, on a warning too.
/// Meant for CI and pre-deploy hooks.
pub(crate) fn check_config(args: &[String]) -> Result<(), CliError> {
    let strict = match args {
        [] => false,
        [flag] if flag == "--strict" => true,
        _ => return Err(usage("Usage: hora check [--strict]")),
    };
    let config_path = config::path();
    let (loaded, warnings) = count_warnings(|| config::load_from(&config_path));
    if let Err(err) = loaded {
        return Err(CliError::Failed(format!("Configuration error: {err:#}")));
    }
    let path = config_path.display();
    let counted = match warnings {
        1 => "1 warning".to_owned(),
        n => format!("{n} warnings"),
    };
    match warnings {
        0 => println!("{path} is valid."),
        _ if strict => {
            return Err(CliError::Failed(format!(
                "{path} is valid but has {counted} (--strict)."
            )));
        }
        _ => println!("{path} is valid, with {counted} (above)."),
    }
    Ok(())
}

/// Run `f` with its warnings and errors logged to stderr, and return how many
/// were logged alongside its result.
pub(crate) fn count_warnings<T>(f: impl FnOnce() -> T) -> (T, usize) {
    use std::io::IsTerminal as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    let counter = WarningCounter::default();
    let count = Arc::clone(&counter.0);
    let print = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .without_time()
        .with_target(false);
    let subscriber = tracing_subscriber::registry()
        .with(print)
        .with(counter)
        .with(tracing_subscriber::filter::LevelFilter::WARN);
    let result = tracing::subscriber::with_default(subscriber, f);
    (result, count.load(Ordering::Relaxed))
}

/// Counts the events it sees (the subscriber filters them to warnings and
/// errors).
#[derive(Default)]
struct WarningCounter(Arc<AtomicUsize>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarningCounter {
    fn on_event(
        &self,
        _event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// Send a test `Down` then `Recovered` through the real notification chain, so
/// an operator verifies delivery *before* the first real incident instead of
/// during it. Without an id every configured channel is exercised; with one,
/// the monitor's `notify` routing applies - testing exactly what would fire.
/// Failures surface as the notifiers' own per-channel warnings *and* as a
/// non-zero exit, so a CI pipeline can gate on the notification chain.
pub(crate) async fn test_alert(monitor_id: Option<&str>) -> Result<(), CliError> {
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
        local_only: false,
    };
    let mut failed = dispatcher.dispatch(event, notify.as_deref()).await;
    let failed_recovery = dispatcher
        .dispatch(
            hora_core::notifications::Event::Recovered {
                monitor: &name,
                local_only: false,
            },
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

/// Snapshot the database to `dest` via `VACUUM INTO`: consistent and compacted,
/// safe while the daemon runs. Meant for cron ("a one-statement answer to 'what
/// if I lose a year of history?'").
pub(crate) async fn backup(dest: &str) -> Result<(), CliError> {
    let config_path = config::path();
    let config = config::load_from(&config_path).context("loading configuration")?;
    let source = &config.server.database_path;
    hora_core::db::backup_into(source, dest).await?;
    let size = std::fs::metadata(dest).map_or(0, |meta| meta.len());
    println!("Backed up {source} to {dest} ({} KiB).", size / 1024);
    Ok(())
}

/// Print the digest exactly as the `[digest]` task would send it - a dry run
/// to check the wording (and the data) without notifying anyone.
pub(crate) async fn digest_preview() -> Result<(), CliError> {
    let (config, store) = open_database().await?;
    let now = chrono::Utc::now().timestamp();
    let (period, summary) = hora_core::digest::build_summary(&store, &config, now).await?;
    println!("Hora digest ({period})");
    println!("{summary}");
    Ok(())
}

/// Diagnose the runtime environment against what the config needs: `hora
/// check` says the config is sound, `hora doctor` says the *host* can honour
/// it. Exits non-zero when a needed capability is missing.
pub(crate) async fn doctor() -> Result<(), CliError> {
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
