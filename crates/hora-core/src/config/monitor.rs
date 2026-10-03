//! A `[[monitors]]` entry and its probe kind.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use super::parsed::{DnsRecord, Parsed};
use super::secret::{Secret, redact_url_secrets};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Http,
    Tcp,
    /// ICMP echo (ping). Target is a host or IP (no port). Uses an unprivileged
    /// datagram socket, so it works in rootless Docker without `CAP_NET_RAW`
    /// (the kernel's `net.ipv4.ping_group_range` must cover the process, which is
    /// the Docker default). IPv4 and IPv6 are both supported.
    Icmp,
    /// External command probe (`command = [...]`), following the
    /// monitoring-plugins convention: exit 0 = up, 1 = degraded, anything
    /// else = down; the first stdout line is the message. Requires the
    /// `HORA_EXEC_DIR` environment variable - the deployment-level consent -
    /// and only runs executables inside that directory.
    Exec,
    /// Passive heartbeat: the monitored job pings `/api/push/{id}`; missing a
    /// heartbeat within the interval marks it down. No active probing.
    Push,
    /// DNS resolution: resolve a name and assert the expected record type/value.
    /// Target is a hostname; `dns_expected` pins the answer (hijack detection).
    Dns,
}

impl Kind {
    /// Lowercase name, as written in the config and the API.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Tcp => "tcp",
            Self::Icmp => "icmp",
            Self::Exec => "exec",
            Self::Push => "push",
            Self::Dns => "dns",
        }
    }
}

/// `release = { github = "owner/repo", current = "v1.2.3" }` on a monitor:
/// which project is watched, and how Hora knows the version that runs.
///
/// The running version is either written down (`current`, the line to edit
/// on each upgrade) or read from the service itself (`current_url`, with
/// `current_query` when the answer is JSON): the second form cannot drift from
/// what is deployed, and clears the alert by itself once the upgrade is done.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseWatch {
    /// The project on GitHub, `owner/repo`. Its latest published release is
    /// what counts: drafts and prereleases are not.
    pub github: String,
    /// The version that runs, as a literal ("v1.9.1", "1.9.1").
    #[serde(default)]
    pub current: Option<String>,
    /// An address of the service that answers its own version: a JSON document
    /// (give `current_query`) or a bare version as text.
    #[serde(default)]
    pub current_url: Option<String>,
    /// `JSONPath` to the version inside the JSON answered by `current_url`.
    #[serde(default)]
    pub current_query: Option<Parsed<serde_json_path::JsonPath>>,
}

impl std::fmt::Debug for ReleaseWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleaseWatch")
            .field("github", &self.github)
            .field("current", &self.current)
            // A version endpoint may sit behind a `?token=`, like a target.
            .field(
                "current_url",
                &self.current_url.as_deref().map(redact_url_secrets),
            )
            .field("current_query", &self.current_query)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Monitor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// Probe target (URL for HTTP, `host:port` for TCP, host or IP for ICMP,
    /// hostname for DNS). Unused for push monitors.
    #[serde(default)]
    pub target: String,
    pub interval_secs: u64,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub expected_status: Option<u16>,
    #[serde(default)]
    pub degraded_over_ms: Option<i64>,
    /// Latency SLO objective in ms: the 24h p95 is flagged met/breached against it.
    #[serde(default)]
    pub slo_latency_ms: Option<i64>,
    /// Availability SLO target in percent (e.g. `99.9`), evaluated over
    /// [`slo_window_days`](Self::slo_window_days). Drives the error-budget
    /// display on the status page and the burn-rate alerts. Stored in basis
    /// points (99.9% = 9990) so the config stays `Eq`-comparable.
    #[serde(default, deserialize_with = "de_slo_uptime")]
    pub slo_uptime: Option<u32>,
    /// Window for the availability SLO, in days (default 30).
    #[serde(default)]
    pub slo_window_days: Option<u16>,
    /// Extra HTTP request headers (e.g. `Accept`, `Authorization`). HTTP monitors
    /// only. Values are redacted in `Debug` (a header may carry a credential).
    #[serde(default)]
    pub headers: HashMap<String, Secret>,
    /// HTTP body assertion: the response body must contain this text.
    #[serde(default)]
    pub keyword: Option<String>,
    /// Invert the [`keyword`](Self::keyword) check: the body must *not* contain it.
    #[serde(default)]
    pub keyword_invert: bool,
    /// HTTP body assertion: a `JSONPath` (RFC 9535) evaluated against a JSON response.
    #[serde(default)]
    pub json_query: Option<Parsed<serde_json_path::JsonPath>>,
    /// Value the [`json_query`](Self::json_query) result must equal (string compare).
    /// When unset, the query only has to match at least one node.
    #[serde(default)]
    pub json_expected: Option<String>,
    /// HTTP body assertion: a regex extracting a number from the response body -
    /// the first capture group when the pattern has one, the whole match
    /// otherwise. Fails when nothing matches, the extracted text is not a
    /// number, or the value falls outside [`number_min`](Self::number_min) /
    /// [`number_max`](Self::number_max). Made for pages that render a gauge
    /// inline ("11 unique ships") with no JSON endpoint behind them.
    #[serde(default)]
    pub number_regex: Option<Parsed<regex::Regex>>,
    /// Lowest value (inclusive) the [`number_regex`](Self::number_regex)
    /// extraction may report before the monitor goes down.
    #[serde(default)]
    pub number_min: Option<i64>,
    /// Highest value (inclusive) the [`number_regex`](Self::number_regex)
    /// extraction may report before the monitor goes down.
    #[serde(default)]
    pub number_max: Option<i64>,
    /// Cap (KiB) on the response body read for keyword/JSON assertions
    /// (default 1024 = 1 MiB). Raise for large JSON endpoints, with care.
    #[serde(default)]
    pub max_body_kb: Option<u32>,
    /// Immediate re-probes after a failed check before recording it (default
    /// 1, max 5). A single network blip between the prober and the target
    /// thus never lands in the history - so the page, the uptime numbers and
    /// the error budget only count failures that survived a retry. Set 0 to
    /// record every raw probe result. Not applicable to push monitors.
    #[serde(default)]
    pub probe_retries: Option<u32>,
    /// Probe both address families and require both to pass - the classic
    /// silent failure is a service whose IPv6 has been dead for weeks behind a
    /// healthy IPv4. One broken family marks the monitor down, naming it in
    /// the reason ("IPv6 failing: connection timed out (IPv4 ok)"). For http,
    /// tcp and icmp monitors with a hostname target; not combinable with
    /// `proxy`. The probing host itself needs working IPv4 *and* IPv6.
    #[serde(default)]
    pub dual_stack: Option<bool>,
    /// Restrict this monitor's alerts to these channel names (e.g. `["ops"]`).
    /// Unset = every configured channel.
    #[serde(default)]
    pub notify: Option<Vec<String>>,
    /// Route this monitor's HTTP requests through a proxy (`http(s)://…` or
    /// `socks5://…`). HTTP monitors only.
    #[serde(default)]
    pub proxy: Option<String>,
    /// Push monitor only: secret required as `?token=` on `/api/push/{id}`.
    /// Unset = no token check (anyone who knows the id can heartbeat).
    #[serde(default)]
    pub push_token: Option<Secret>,
    /// Push monitor only: a five-field cron expression (UTC) for when heartbeats
    /// are expected (e.g. `"0 3 * * *"` for a nightly job). The monitor goes
    /// down only when a scheduled run misses its [`grace_secs`](Self::grace_secs)
    /// window - `interval_secs` then merely sets how often Hora re-evaluates.
    #[serde(default)]
    pub schedule: Option<String>,
    /// Push monitor with a [`schedule`](Self::schedule): how late (seconds) a
    /// heartbeat may arrive after its scheduled time before alerting.
    /// Default 1800 (30 minutes).
    #[serde(default)]
    pub grace_secs: Option<u64>,
    /// Override TLS certificate checking. Defaults to on for `https://` HTTP
    /// monitors, and for tcp monitors with [`starttls`](Self::starttls) set.
    #[serde(default)]
    pub check_cert: Option<bool>,
    /// TCP monitor: negotiate `STARTTLS` (`"smtp"` or `"imap"`) before the
    /// certificate check, so the expiry/pin machinery covers mail servers on
    /// 587/143 - the certificate nobody ever looks at until it expires. The
    /// regular tcp probe is unchanged (a connect is still the up/down signal);
    /// only the certificate watcher upgrades the connection.
    #[serde(default)]
    pub starttls: Option<String>,
    /// `starttls = "smtp"`: the name announced in `EHLO`. RFC 5321 wants a
    /// fully qualified domain or an address literal, and MX servers enforce it
    /// on port 25 (`550 Invalid EHLO domain`). Default: the address literal of
    /// the connection's local end (`[192.0.2.10]`), valid everywhere,
    /// including inside a container whose hostname is a bare id.
    #[serde(default)]
    pub ehlo_name: Option<String>,
    /// Override how long this monitor's checks are kept before pruning.
    #[serde(default)]
    pub retention_days: Option<u16>,
    /// Display group for the status page (monitors sharing a group are rendered
    /// together under a section header). Ungrouped monitors appear last.
    #[serde(default)]
    pub group: Option<String>,
    /// Upstream monitor ids this one depends on. When set, a down alert is
    /// annotated with the probable upstream cause (if any upstream is also
    /// down) or with the dependents it impacts (if this is a root cause).
    /// Validated as a DAG at load time.
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    /// Whether this monitor appears on the public status page. `true` by default;
    /// set `false` to hide it from unauthenticated viewers (requires a token).
    #[serde(default = "default_true")]
    pub public: bool,
    /// Show this monitor's full failure detail to anonymous viewers. By default
    /// the public page collapses failure reasons to a safe category ("HTTP 500",
    /// "content check failed"), since the stored detail can carry response-body
    /// snippets, DNS answers or asserted keywords. Opt in per monitor when that
    /// detail is fine to publish (e.g. a push monitor whose `msg` is meant for
    /// the page). Authenticated viewers always see the full reason.
    #[serde(default)]
    pub public_error_detail: bool,
    /// DNS monitor: the record type to query (e.g. `"A"`, `"AAAA"`, `"CNAME"`),
    /// case-insensitive. Unset = `A`.
    #[serde(default)]
    pub dns_record: Option<Parsed<DnsRecord>>,
    /// DNS monitor: the expected answer, comma-separated when several records
    /// are pinned, compared order-insensitively (hijack detection). When unset,
    /// any non-empty answer counts as up - no change detection, since answers
    /// rotate freely behind CDNs and round-robin records.
    #[serde(default)]
    pub dns_expected: Option<String>,
    /// DNS monitor: custom resolver address, an IP and a port (`9.9.9.9:53`,
    /// `[2620:fe::fe]:53`). Unset = system default. A hostname is refused: the
    /// resolver's own address can't be looked up through itself.
    #[serde(default)]
    pub dns_resolver: Option<Parsed<std::net::SocketAddr>>,
    /// TLS certificate pinning: hex-encoded SHA-256 of the leaf public key. When
    /// set, an alert fires if the key changes (MITM / unexpected renewal detection).
    #[serde(default)]
    pub cert_pin: Option<String>,
    /// The monitor's *registered* domain ("example.com", not the subdomain the
    /// target uses), checked daily against the registry over RDAP; an alert
    /// fires `alerts.domain_expiry_days` before it expires. Explicit rather
    /// than derived from `target`: registrable-domain extraction needs a
    /// public-suffix list, and the operator already knows the answer.
    #[serde(default)]
    pub domain_expiry: Option<String>,
    /// Watch the upstream releases of the software behind this monitor: an
    /// alert fires, once per release, when one newer than the version that
    /// runs is published. See [`ReleaseWatch`].
    #[serde(default)]
    pub release: Option<ReleaseWatch>,
    /// Multi-vantage confirmation override for this monitor; unset = the
    /// `[health].confirm_with_peers` default. See that field for semantics.
    #[serde(default)]
    pub confirm_with_peers: Option<bool>,
    /// Exec monitors: the argv to run - `["check_raid", "--no-sudo"]`. No
    /// shell is involved, and `command[0]` is resolved strictly inside
    /// `HORA_EXEC_DIR` (a bare file name, no path separators).
    #[serde(default)]
    pub command: Vec<String>,
}

// Manual `Debug` (rather than derived) so credentials never leak: `target` and
// `proxy` may embed `user:pass@` or a `?token=` query, and `headers`/`push_token`
// are `Secret` (which self-redact). A `{:?}` of a `Monitor` - or of the whole
// `Config`, which derives `Debug` and holds a `Vec<Monitor>` - is therefore safe
// to log.
impl std::fmt::Debug for Monitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Monitor")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("target", &redact_url_secrets(&self.target))
            .field("interval_secs", &self.interval_secs)
            .field("timeout_secs", &self.timeout_secs)
            .field("expected_status", &self.expected_status)
            .field("degraded_over_ms", &self.degraded_over_ms)
            .field("slo_latency_ms", &self.slo_latency_ms)
            .field("slo_uptime", &self.slo_uptime)
            .field("slo_window_days", &self.slo_window_days)
            .field("headers", &self.headers)
            .field("keyword", &self.keyword)
            .field("keyword_invert", &self.keyword_invert)
            .field("json_query", &self.json_query)
            .field("json_expected", &self.json_expected)
            .field("number_regex", &self.number_regex)
            .field("number_min", &self.number_min)
            .field("number_max", &self.number_max)
            .field("max_body_kb", &self.max_body_kb)
            .field("probe_retries", &self.probe_retries)
            .field("dual_stack", &self.dual_stack)
            .field("notify", &self.notify)
            .field("proxy", &self.proxy.as_deref().map(redact_url_secrets))
            .field("push_token", &self.push_token)
            .field("schedule", &self.schedule)
            .field("grace_secs", &self.grace_secs)
            .field("check_cert", &self.check_cert)
            .field("starttls", &self.starttls)
            .field("ehlo_name", &self.ehlo_name)
            .field("retention_days", &self.retention_days)
            .field("group", &self.group)
            .field("depends_on", &self.depends_on)
            .field("public", &self.public)
            .field("public_error_detail", &self.public_error_detail)
            .field("dns_record", &self.dns_record)
            .field("dns_expected", &self.dns_expected)
            .field("dns_resolver", &self.dns_resolver)
            .field("cert_pin", &self.cert_pin)
            .field("domain_expiry", &self.domain_expiry)
            .field("release", &self.release)
            .field("confirm_with_peers", &self.confirm_with_peers)
            // Arguments may carry credentials (a plugin's API key): name only.
            .field("command", &self.command.first())
            .finish()
    }
}

impl Monitor {
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    #[must_use]
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }

    /// Whether this monitor probes both address families (default off).
    #[must_use]
    pub fn dual_stack(&self) -> bool {
        self.dual_stack.unwrap_or(false)
    }

    /// Whether this monitor should have its TLS certificate expiry checked:
    /// on by default for `https://` HTTP monitors and for tcp monitors with
    /// `starttls` set, overridable either way with `check_cert`.
    #[must_use]
    pub fn checks_cert(&self) -> bool {
        self.check_cert.unwrap_or_else(|| {
            (self.kind == Kind::Http && self.target.starts_with("https://"))
                || (self.kind == Kind::Tcp && self.starttls.is_some())
        })
    }

    /// Effective storage retention in days, falling back to the global default.
    #[must_use]
    pub fn retention_days(&self, default: u16) -> u16 {
        self.retention_days.unwrap_or(default)
    }

    /// Byte cap for assertion body reads (falls back to the 1 MiB default).
    #[must_use]
    pub fn assertion_body_cap(&self) -> usize {
        const DEFAULT: usize = 1 << 20; // 1 MiB
        self.max_body_kb.map_or(DEFAULT, |kb| {
            usize::try_from(kb)
                .unwrap_or(usize::MAX)
                .saturating_mul(1024)
        })
    }

    /// Grace period for a scheduled push monitor (default 30 minutes).
    #[must_use]
    pub fn push_grace_secs(&self) -> u64 {
        self.grace_secs.unwrap_or(1800)
    }

    /// Immediate re-probes after a failure before recording it (default 1).
    #[must_use]
    pub fn probe_retries(&self) -> u32 {
        self.probe_retries.unwrap_or(1)
    }

    /// Availability SLO window in days (default 30).
    #[must_use]
    pub fn slo_window_days(&self) -> u16 {
        self.slo_window_days.unwrap_or(30)
    }

    /// A throwaway monitor for a one-shot `hora probe`: a target and a kind,
    /// every other setting at its default. It never reaches the scheduler, the
    /// database or the config file, so its id, name and interval are cosmetic;
    /// only `kind` + `target` drive [`crate::probe::run`].
    #[must_use]
    pub fn ad_hoc(kind: Kind, target: String) -> Self {
        Self {
            id: "probe".to_owned(),
            name: target.clone(),
            kind,
            target,
            // A one-shot ad-hoc probe wants the honest first result, not the
            // anti-flap retry (the peer responder does the same for confirm).
            probe_retries: Some(0),
            ..Self::default()
        }
    }
}

/// Every field at the value an omitted TOML key gets (`interval_secs`, which
/// the file must spell out, at a minute). The base for ad-hoc monitors and
/// test fixtures, so a new field is added in one place.
impl Default for Monitor {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: Kind::default(),
            target: String::new(),
            interval_secs: 60,
            timeout_secs: default_timeout(),
            expected_status: None,
            degraded_over_ms: None,
            slo_latency_ms: None,
            slo_uptime: None,
            slo_window_days: None,
            headers: HashMap::new(),
            keyword: None,
            keyword_invert: false,
            json_query: None,
            json_expected: None,
            number_regex: None,
            number_min: None,
            number_max: None,
            max_body_kb: None,
            probe_retries: None,
            dual_stack: None,
            notify: None,
            proxy: None,
            push_token: None,
            schedule: None,
            grace_secs: None,
            check_cert: None,
            starttls: None,
            ehlo_name: None,
            retention_days: None,
            group: None,
            depends_on: None,
            public: default_true(),
            public_error_detail: false,
            dns_record: None,
            dns_expected: None,
            dns_resolver: None,
            cert_pin: None,
            domain_expiry: None,
            release: None,
            confirm_with_peers: None,
            command: Vec::new(),
        }
    }
}

/// Split `host:port` or `[ipv6]:port` into a non-empty host (brackets
/// stripped) and a numeric port - the one parser behind every `host:port`
/// field, so they all agree on IPv6. An unbracketed address with more than
/// one colon (`::1:80`) is ambiguous - is `80` a port or the last group of the
/// address? - and returns `None`, as does a bracketed host that is not an IPv6
/// literal.
#[must_use]
pub fn split_host_port(target: &str) -> Option<(&str, u16)> {
    let (host, port) = if let Some(rest) = target.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return None;
        }
        (host, after.strip_prefix(':')?)
    } else {
        let (host, port) = target.rsplit_once(':')?;
        if host.contains(':') {
            return None;
        }
        (host, port)
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port.parse().ok()?))
}

/// Accept the availability SLO as a percent float (`99.9`) and store basis
/// points (9990): the config structs stay `Eq` and no precision is lost for
/// the targets that occur in practice (two decimals).
fn de_slo_uptime<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(percent) = Option::<f64>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if !percent.is_finite() || percent <= 0.0 || percent >= 100.0 {
        return Err(serde::de::Error::custom(
            "slo_uptime must be a percentage between 0 and 100 (exclusive), e.g. 99.9",
        ));
    }
    // Lossless float-to-int without a cast: the value is bounded (0, 10_000)
    // by the guard above, and this runs once at config load.
    let basis_points = format!("{:.0}", percent * 100.0)
        .parse::<u32>()
        .map_err(serde::de::Error::custom)?;
    Ok(Some(basis_points.clamp(1, 9999)))
}

pub(super) fn default_timeout() -> u64 {
    10
}
fn default_true() -> bool {
    true
}
