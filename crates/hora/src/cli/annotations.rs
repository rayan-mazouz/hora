//! Operator annotations from the shell: announcements, event markers and
//! silences.

use hora_core::announce::{self, Announcement};
use hora_core::silence::SilenceError;
use hora_core::{MAX_EVENT_TITLE_CHARS, fmt};

use super::{CliError, open_database, unknown_monitor, usage};

/// `hora announce`: pin (or list/clear) a public banner on the status page -
/// the communication side of incidents, written straight into the daemon's
/// database and live within the summary cache TTL (~5s). The HTTP twin is
/// `POST /api/announce`. A leading `--` ends the subcommand words, so a banner
/// titled "clear skies" is `hora announce -- clear skies`.
pub(crate) async fn announce(args: &[String]) -> Result<(), CliError> {
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
pub(super) fn parse_announce_args(args: &[String], now: i64) -> anyhow::Result<Announcement> {
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

/// `hora event <title...>` / `hora event list [N]`: record (or list) an event
/// marker - "deploy api v2.3" - the same gesture as a silence, written
/// straight into the daemon's database. Markers overlay the latency
/// sparklines, list on /history, and a down confirming within the hour is
/// annotated "recent change: ...". The HTTP twin is `POST /api/event`.
pub(crate) async fn event(args: &[String]) -> Result<(), CliError> {
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
pub(super) enum EventCommand {
    /// `list [limit]`.
    List(i64),
    /// Record a marker with this title (trimmed, at most 200 chars).
    Record(String),
}

/// Parse `hora event` arguments. `list` is the subcommand only when followed
/// by nothing or a number: `hora event list deploy` is a usage error that
/// points at `--` rather than guessing - a leading `--` ends the subcommand
/// words, so `hora event -- list deploy` records "list deploy".
pub(super) fn parse_event_args(args: &[String]) -> Result<EventCommand, CliError> {
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

/// `hora silence <ids|all> <duration> [reason]` / `list` / `clear`: ad-hoc
/// alert muting (a deploy window) written straight into the daemon's database,
/// picked up on its next tick. The HTTP counterpart is `POST /api/silence`.
pub(crate) async fn silence(args: &[String]) -> Result<(), CliError> {
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
