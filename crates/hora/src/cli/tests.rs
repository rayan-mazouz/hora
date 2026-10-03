use hora_core::MAX_EVENT_TITLE_CHARS;
use hora_core::announce;

use super::annotations::{EventCommand, parse_announce_args, parse_event_args};
use super::probe::parse_probe_args;
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
    let store = hora_core::db::Store::in_memory().await;
    assert_eq!(resolve_incident_id(&store, "42").await.expect("number"), 42);
    assert!(
        usage_message(resolve_incident_id(&store, "4x2").await).contains("a number, or 'last'")
    );
    match resolve_incident_id(&store, "last").await {
        Err(CliError::Failed(message)) => assert_eq!(message, "No incidents recorded yet."),
        other => panic!("expected a failure on an empty database, got {other:?}"),
    }
}
