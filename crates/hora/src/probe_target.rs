//! `hora probe` target inference: what kind of check a bare target means.

use hora_core::config::{Kind, split_host_port};

/// Classify a bare `hora probe` target (no `--kind`) into a kind and a
/// normalized target:
///
/// - an `http(s)://` URL is an HTTP check, kept as-is;
/// - a bare IP is an ICMP ping (a host to reach), even IPv6 (all colons);
/// - an explicit `host:port` or `[ipv6]:port` is a TCP connect;
/// - a bare **hostname** is an HTTPS check (`https://` prepended): a name
///   denotes a web service far more often than a ping target, and ICMP is
///   widely filtered, so a ping default would cry wolf.
///
/// DNS is never inferred - it needs a record type - so use `--kind dns`.
#[must_use]
pub fn infer_probe(raw: &str) -> (Kind, String) {
    let raw = raw.trim();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return (Kind::Http, raw.to_owned());
    }
    if raw.parse::<std::net::IpAddr>().is_ok() {
        return (Kind::Icmp, raw.to_owned());
    }
    if split_host_port(raw).is_some() {
        return (Kind::Tcp, raw.to_owned());
    }
    (Kind::Http, format!("https://{raw}"))
}

/// The target for an explicit `--kind`: an `http` kind without a scheme gets
/// `https://` prepended so it is a valid URL; every other kind takes the raw
/// target unchanged.
#[must_use]
pub fn probe_target(kind: Kind, raw: &str) -> String {
    let raw = raw.trim();
    if kind == Kind::Http && !raw.starts_with("http://") && !raw.starts_with("https://") {
        format!("https://{raw}")
    } else {
        raw.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_probe_classifies_targets_and_normalizes() {
        // URLs are HTTP, scheme wins over anything that follows, kept as-is.
        assert_eq!(
            infer_probe("https://example.com"),
            (Kind::Http, "https://example.com".to_owned())
        );
        assert_eq!(
            infer_probe("http://example.com:8080/health"),
            (Kind::Http, "http://example.com:8080/health".to_owned())
        );
        // host:port (and bracketed ipv6:port) is a TCP connect.
        assert_eq!(
            infer_probe("db.example.com:5432"),
            (Kind::Tcp, "db.example.com:5432".to_owned())
        );
        assert_eq!(
            infer_probe("[2001:db8::1]:443"),
            (Kind::Tcp, "[2001:db8::1]:443".to_owned())
        );
        // A bare IP is a ping target - including unbracketed IPv6.
        assert_eq!(
            infer_probe("192.168.1.10"),
            (Kind::Icmp, "192.168.1.10".to_owned())
        );
        assert_eq!(infer_probe("::1"), (Kind::Icmp, "::1".to_owned()));
        // A bare hostname is an HTTPS check, scheme prepended.
        assert_eq!(
            infer_probe("example.com"),
            (Kind::Http, "https://example.com".to_owned())
        );
        // A non-numeric "port" is not host:port: it's a (weird) hostname -> https.
        assert_eq!(
            infer_probe("example.com:http"),
            (Kind::Http, "https://example.com:http".to_owned())
        );
    }

    #[test]
    fn probe_target_prepends_https_only_for_schemeless_http() {
        assert_eq!(
            probe_target(Kind::Http, "example.com"),
            "https://example.com"
        );
        assert_eq!(
            probe_target(Kind::Http, "http://example.com"),
            "http://example.com"
        );
        // Other kinds take the target verbatim.
        assert_eq!(probe_target(Kind::Tcp, "db:5432"), "db:5432");
        assert_eq!(probe_target(Kind::Icmp, "example.com"), "example.com");
    }
}
