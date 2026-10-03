//! Monitor fields parsed once at load (regexes, JSON paths, addresses, DNS
//! record types) rather than on every probe.

use serde::Deserialize;

use super::split_host_port;

/// A config string parsed once, at load, into the typed value the probe uses
/// on every tick (a compiled regex, a `JSONPath`, a socket address) instead of
/// re-parsing it per probe.
///
/// A malformed value does *not* fail deserialization: the error is kept and
/// reported by validation, which can name the monitor - serde's error path
/// only says `monitors.number_regex`, not which `[[monitors]]` entry is wrong.
/// Equality (hot-reload change detection) and `Debug` go by the raw text.
#[derive(Clone)]
pub struct Parsed<T> {
    raw: String,
    value: Result<T, String>,
}

/// How a [`Parsed`] field turns its raw text into a value.
pub trait ParseField: Sized {
    /// Parse the raw config text.
    ///
    /// # Errors
    ///
    /// A human-readable reason (validation prefixes the monitor and field)
    /// when `raw` is not a valid value.
    fn parse_field(raw: &str) -> Result<Self, String>;
}

impl<T: ParseField> Parsed<T> {
    /// Parse `raw` now, keeping the error (if any) for validation.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let value = T::parse_field(&raw);
        Self { raw, value }
    }
}

impl<T> Parsed<T> {
    /// The text as written in the config.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The parsed value; `None` only for a malformed value, which validation
    /// rejects before any probe runs.
    #[must_use]
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref().ok()
    }

    /// Why the raw text did not parse, if it did not.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.value.as_ref().err().map(String::as_str)
    }
}

impl<T: Clone> Parsed<T> {
    /// The parsed value with its text, once validation has made sure it
    /// parsed; `None` for a malformed value.
    #[must_use]
    pub fn compiled(&self) -> Option<Compiled<T>> {
        self.get().map(|value| Compiled {
            raw: self.raw.clone(),
            value: value.clone(),
        })
    }
}

/// A config string and the value it parsed into, after validation: unlike a
/// [`Parsed`], it cannot hold an error, so the probes use the value directly.
/// Equality (hot-reload change detection) and `Debug` go by the raw text.
#[derive(Clone)]
pub struct Compiled<T> {
    raw: String,
    value: T,
}

impl<T> Compiled<T> {
    /// The text as written in the config.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }
}

impl<T: ParseField> Compiled<T> {
    /// Parse `raw`, for values built outside a config file (tests, the CLI).
    ///
    /// # Errors
    ///
    /// The parse error, as for a config value.
    pub fn parse(raw: &str) -> Result<Self, String> {
        T::parse_field(raw).map(|value| Self {
            raw: raw.to_owned(),
            value,
        })
    }
}

impl<T> std::ops::Deref for Compiled<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> PartialEq for Compiled<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for Compiled<T> {}

impl<T> std::fmt::Debug for Compiled<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.raw, f)
    }
}

impl<T> PartialEq for Parsed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for Parsed<T> {}

impl<T> std::fmt::Debug for Parsed<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.raw, f)
    }
}

impl<'de, T: ParseField> Deserialize<'de> for Parsed<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

impl ParseField for regex::Regex {
    fn parse_field(raw: &str) -> Result<Self, String> {
        Self::new(raw).map_err(|err| err.to_string())
    }
}

impl ParseField for croner::Cron {
    fn parse_field(raw: &str) -> Result<Self, String> {
        super::parse_cron(raw).map_err(|err| err.to_string())
    }
}

impl ParseField for serde_json_path::JsonPath {
    fn parse_field(raw: &str) -> Result<Self, String> {
        Self::parse(raw).map_err(|err| err.to_string())
    }
}

/// A DNS resolver address: an IP and a port, IPv6 bracketed. A hostname is
/// refused rather than resolved at load: it would be looked up through some
/// *other* resolver, and silently go stale when its address changes.
impl ParseField for std::net::SocketAddr {
    fn parse_field(raw: &str) -> Result<Self, String> {
        if let Ok(addr) = raw.parse() {
            return Ok(addr);
        }
        Err(if split_host_port(raw).is_some() {
            "must be an IP address and port such as 9.9.9.9:53 or [2620:fe::fe]:53, not a hostname"
        } else {
            "must be host:port, e.g. 9.9.9.9:53 or [2620:fe::fe]:53"
        }
        .to_owned())
    }
}

/// The DNS record types a dns monitor can query: the one list shared by the
/// config, the probe and the public failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsRecord {
    A,
    Aaaa,
    Cname,
    Mx,
    Ns,
    Txt,
    Srv,
    Soa,
    Ptr,
}

impl DnsRecord {
    /// Every supported type.
    pub const ALL: [Self; 9] = [
        Self::A,
        Self::Aaaa,
        Self::Cname,
        Self::Mx,
        Self::Ns,
        Self::Txt,
        Self::Srv,
        Self::Soa,
        Self::Ptr,
    ];

    /// Uppercase name, as DNS tools (and the probe's failure reasons) spell it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Cname => "CNAME",
            Self::Mx => "MX",
            Self::Ns => "NS",
            Self::Txt => "TXT",
            Self::Srv => "SRV",
            Self::Soa => "SOA",
            Self::Ptr => "PTR",
        }
    }
}

impl ParseField for DnsRecord {
    fn parse_field(raw: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|record| record.as_str().eq_ignore_ascii_case(raw.trim()))
            .ok_or_else(|| format!("unsupported dns_record type {raw:?}"))
    }
}
