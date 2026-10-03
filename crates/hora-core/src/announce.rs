//! Ad-hoc status-page announcements ("Fiber cut, ETA 18:00"): one validation
//! and one flag grammar for the three doors that pin them - `POST
//! /api/announce`, `hora announce` and the `a` key of `hora top` - so a banner
//! reads, and is refused, the same way whichever one it came through.

use crate::db::Store;

pub use crate::config::{Severity, UnknownSeverity};
use crate::{MAX_ANNOUNCE_BODY_CHARS, MAX_ANNOUNCE_TITLE_CHARS, SECONDS_PER_DAY, bounded, db};

/// Why an announcement was refused. [`AnnounceError::message`] is the wording
/// every door answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnounceError {
    /// The title is empty once trimmed.
    EmptyTitle,
    /// A severity outside `info`, `warning`, `critical` and `resolved`.
    UnknownSeverity,
    /// An expiry that is neither a duration (`4h`) nor a UTC time (`18:00`).
    InvalidUntil,
    /// `--severity` or `--until` as the last word, without its value.
    MissingValue(&'static str),
}

impl AnnounceError {
    /// The human-facing reason.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::EmptyTitle => "title must not be empty",
            Self::UnknownSeverity => UnknownSeverity::MESSAGE,
            Self::InvalidUntil => "invalid until (use a duration like 4h, or HH:MM UTC)",
            Self::MissingValue("--severity") => "--severity needs a value",
            Self::MissingValue(_) => "--until needs a value (e.g. 4h or 18:00)",
        }
    }
}

impl std::fmt::Display for AnnounceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for AnnounceError {}

/// A validated announcement, ready to pin: title and body trimmed and capped
/// ([`MAX_ANNOUNCE_TITLE_CHARS`], [`MAX_ANNOUNCE_BODY_CHARS`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub title: String,
    pub body: String,
    pub severity: Severity,
    /// Auto-expiry (unix epoch seconds, UTC); `None` = until cleared.
    pub until: Option<i64>,
}

impl Announcement {
    /// Validate and bound an announcement.
    ///
    /// # Errors
    ///
    /// [`AnnounceError::EmptyTitle`] when the title is blank.
    pub fn new(
        title: &str,
        body: &str,
        severity: Severity,
        until: Option<i64>,
    ) -> Result<Self, AnnounceError> {
        let title = bounded(title, MAX_ANNOUNCE_TITLE_CHARS);
        if title.is_empty() {
            return Err(AnnounceError::EmptyTitle);
        }
        Ok(Self {
            title,
            body: bounded(body, MAX_ANNOUNCE_BODY_CHARS),
            severity,
            until,
        })
    }

    /// Pin it as a status-page banner, returning its id.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub async fn pin(&self, store: &Store) -> crate::db::Result<i64> {
        db::insert_announcement(
            store,
            &self.title,
            &self.body,
            self.severity.as_str(),
            self.until,
        )
        .await
    }
}

/// Parse a severity name.
///
/// # Errors
///
/// [`AnnounceError::UnknownSeverity`] for anything but the four names.
pub fn parse_severity(raw: &str) -> Result<Severity, AnnounceError> {
    raw.parse()
        .map_err(|UnknownSeverity| AnnounceError::UnknownSeverity)
}

/// An expiry: a duration from `now` (`4h`, `90m`), or a UTC clock time
/// (`18:00`) meaning its next occurrence - today if still ahead, tomorrow
/// otherwise. `None` for anything else.
#[must_use]
pub fn parse_until(raw: &str, now: i64) -> Option<i64> {
    let raw = raw.trim();
    if let Some(secs) = crate::parse_duration(raw) {
        return Some(now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX)));
    }
    let (hours, minutes) = raw.split_once(':')?;
    let (hours, minutes): (i64, i64) = (hours.parse().ok()?, minutes.parse().ok()?);
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) {
        return None;
    }
    let today = now.div_euclid(SECONDS_PER_DAY) * SECONDS_PER_DAY + hours * 3600 + minutes * 60;
    Some(if today > now {
        today
    } else {
        today + SECONDS_PER_DAY
    })
}

/// The words of a `hora announce` / `hora top` announcement line with its
/// `--severity <s>` and `--until <4h|18:00>` flags (accepted anywhere) taken
/// out. How the remaining words split into title and body is the caller's
/// grammar.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flags<'a> {
    /// The free words, in order.
    pub words: Vec<&'a str>,
    /// `--severity`, when given.
    pub severity: Option<Severity>,
    /// `--until`, raw (validated: [`parse_until`] accepts it).
    pub until: Option<&'a str>,
}

/// Split the flags out of an announcement line.
///
/// # Errors
///
/// An unknown severity, an unparseable `--until`, or a flag without a value.
pub fn split_flags<'a>(
    words: impl IntoIterator<Item = &'a str>,
) -> Result<Flags<'a>, AnnounceError> {
    let mut flags = Flags::default();
    let mut words = words.into_iter();
    while let Some(word) = words.next() {
        match word {
            "--severity" => {
                let value = words
                    .next()
                    .ok_or(AnnounceError::MissingValue("--severity"))?;
                flags.severity = Some(parse_severity(value)?);
            }
            "--until" => {
                let value = words.next().ok_or(AnnounceError::MissingValue("--until"))?;
                // Any `now` will do: only the shape is checked here.
                parse_until(value, 0).ok_or(AnnounceError::InvalidUntil)?;
                flags.until = Some(value);
            }
            word => flags.words.push(word),
        }
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_takes_a_duration_or_the_next_utc_clock_time() {
        let now = 10 * SECONDS_PER_DAY + 12 * 3600; // 12:00 UTC
        assert_eq!(parse_until("4h", now), Some(now + 4 * 3600));
        assert_eq!(
            parse_until("18:00", now),
            Some(10 * SECONDS_PER_DAY + 18 * 3600)
        );
        // Already past today: tomorrow's.
        assert_eq!(
            parse_until("09:30", now),
            Some(11 * SECONDS_PER_DAY + 9 * 3600 + 1800)
        );
        assert_eq!(
            parse_until("12:00", now),
            Some(11 * SECONDS_PER_DAY + 12 * 3600)
        );
        for bad in ["24:00", "18:60", "-1:00", "noon", "", "4"] {
            assert_eq!(parse_until(bad, now), None, "{bad}");
        }
    }

    #[test]
    fn announcements_are_trimmed_capped_and_need_a_title() {
        let long = "x".repeat(MAX_ANNOUNCE_TITLE_CHARS + 10);
        let announcement =
            Announcement::new(&format!("  {long} "), " body ", Severity::Info, None).unwrap();
        assert_eq!(announcement.title.chars().count(), MAX_ANNOUNCE_TITLE_CHARS);
        assert_eq!(announcement.body, "body");
        assert_eq!(
            Announcement::new("  ", "body", Severity::Info, None),
            Err(AnnounceError::EmptyTitle)
        );
    }

    #[test]
    fn flags_come_out_anywhere_and_are_validated() {
        let flags =
            split_flags("Fiber --severity critical cut --until 18:00 :: ETA".split(' ')).unwrap();
        assert_eq!(flags.words, ["Fiber", "cut", "::", "ETA"]);
        assert_eq!(flags.severity, Some(Severity::Critical));
        assert_eq!(flags.until, Some("18:00"));

        assert_eq!(
            split_flags(["t", "--severity", "panic"]),
            Err(AnnounceError::UnknownSeverity)
        );
        assert_eq!(
            split_flags(["t", "--until", "soon"]),
            Err(AnnounceError::InvalidUntil)
        );
        assert_eq!(
            split_flags(["t", "--until"]),
            Err(AnnounceError::MissingValue("--until"))
        );
        assert_eq!("warning".parse::<Severity>(), Ok(Severity::Warning));
        assert_eq!(Severity::Resolved.to_string(), "resolved");
    }
}
