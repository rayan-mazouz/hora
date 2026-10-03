//! Shared human-facing formatting: durations, timestamps, percentages and the
//! two escapings the pages need. One implementation each, so the history page,
//! the post-mortem, the monthly report and the CLI word the same incident the
//! same way.
//!
//! Durations use a single format everywhere: the two largest units, without
//! padding (`"42s"`, `"3m 10s"`, `"2h 5m"`). [`elapsed`] is the deliberately
//! coarser variant for estimates and "failing for ..." lines, where seconds
//! would be noise.

use std::fmt::Write as _;

use chrono::DateTime;

/// `"42s"`, `"3m 10s"`, `"2h 5m"`: an exact span (incident duration, downtime,
/// MTTR, silence length). Negative spans clamp to zero.
#[must_use]
pub fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

/// `"2d 3h"`, `"6h"`, `"45m"`, `"30s"`: a coarse span, for estimates and
/// "failing for ..." phrasing where the exact seconds are noise.
#[must_use]
pub fn elapsed(seconds: u64) -> String {
    if seconds >= 2 * 86_400 {
        format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3600)
    } else if seconds >= 3600 {
        format!("{}h", seconds / 3600)
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// `"2026-05-14 08:03:12 UTC"`. An out-of-range timestamp prints as the raw
/// number rather than vanishing.
#[must_use]
pub fn utc(timestamp: i64) -> String {
    format_timestamp(timestamp, "%Y-%m-%d %H:%M:%S UTC")
}

/// `"2026-05-14"` (UTC).
#[must_use]
pub fn date(timestamp: i64) -> String {
    format_timestamp(timestamp, "%Y-%m-%d")
}

/// `"May 14"` (UTC): the short form for a period that never spans years.
#[must_use]
pub fn short_date(timestamp: i64) -> String {
    format_timestamp(timestamp, "%b %d")
}

fn format_timestamp(timestamp: i64, pattern: &str) -> String {
    DateTime::from_timestamp(timestamp, 0).map_or_else(
        || timestamp.to_string(),
        |at| at.format(pattern).to_string(),
    )
}

/// The share `part / total` in basis points (9 997 = 99.97%), rounded to the
/// nearest one, in integer math so the result is exact. `None` without data.
#[must_use]
pub fn basis_points(part: i64, total: i64) -> Option<i64> {
    (total > 0).then(|| (part * 10_000 + total / 2) / total)
}

/// `"99.97%"` from basis points.
#[must_use]
pub fn pct_bp(basis_points: i64) -> String {
    let sign = if basis_points < 0 { "-" } else { "" };
    let magnitude = basis_points.unsigned_abs();
    format!("{sign}{}.{:02}%", magnitude / 100, magnitude % 100)
}

/// `"99.97%"` for `part / total`, or `"no checks"` when there is no data.
#[must_use]
pub fn pct(part: i64, total: i64) -> String {
    basis_points(part, total).map_or_else(|| "no checks".to_owned(), pct_bp)
}

/// Escape the five XML metacharacters, for text embedded in SVG, Atom or HTML.
#[must_use]
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Percent-encode `value` for a URL component (RFC 3986): every byte outside
/// the unreserved set becomes `%XX`, uppercase hex.
#[must_use]
pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_keep_the_two_largest_units() {
        assert_eq!(duration(-5), "0s");
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(60), "1m 0s");
        assert_eq!(duration(190), "3m 10s");
        assert_eq!(duration(7500), "2h 5m");
        assert_eq!(duration(50 * 3600), "50h 0m");
    }

    #[test]
    fn elapsed_is_coarse() {
        assert_eq!(elapsed(30), "30s");
        assert_eq!(elapsed(45 * 60 + 59), "45m");
        assert_eq!(elapsed(6 * 3600 + 1800), "6h");
        assert_eq!(elapsed(86_400 + 3600), "25h");
        assert_eq!(elapsed(2 * 86_400 + 3 * 3600), "2d 3h");
    }

    #[test]
    fn timestamps_are_utc() {
        assert_eq!(utc(1_700_000_540), "2023-11-14 22:22:20 UTC");
        assert_eq!(date(1_700_000_540), "2023-11-14");
        assert_eq!(short_date(1_700_000_540), "Nov 14");
        // Out of chrono's range: the raw number, not an empty string.
        assert_eq!(utc(i64::MAX), i64::MAX.to_string());
    }

    #[test]
    fn percentages_use_integer_math() {
        assert_eq!(basis_points(0, 0), None);
        assert_eq!(basis_points(1, 3), Some(3333));
        assert_eq!(basis_points(2, 3), Some(6667));
        assert_eq!(pct_bp(10_000), "100.00%");
        assert_eq!(pct_bp(9_997), "99.97%");
        assert_eq!(pct_bp(5), "0.05%");
        assert_eq!(pct_bp(-150), "-1.50%");
        assert_eq!(pct(0, 0), "no checks");
        assert_eq!(pct(9997, 10_000), "99.97%");
    }

    #[test]
    fn escaping_and_encoding() {
        assert_eq!(
            xml_escape(r#"<a href="x">Tom & 'Jerry'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; &apos;Jerry&apos;&lt;/a&gt;"
        );
        assert_eq!(percent_encode("a b/c~d-é"), "a%20b%2Fc~d-%C3%A9");
    }
}
