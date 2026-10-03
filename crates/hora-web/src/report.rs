//! The monthly service report (`/report/2026-05`): a printable sheet per
//! month - the month at a glance, then a table per group with uptime, the
//! target and whether it was met, incidents, downtime, time to recover and
//! the allowed downtime left. Pre-formatted here; Askama does the escaping.

use askama::Template;
use chrono::{Datelike, NaiveDate, Utc};
use hora_core::fmt;
use hora_core::report::{MonitorMonth, MonthReport};

use crate::layout::{Chrome, join_and, minutes};

#[derive(Template)]
#[template(path = "report.html")]
pub(crate) struct ReportTemplate {
    pub(crate) chrome: Chrome,
    /// Whose report this is (the page title, `· group` when scoped).
    pub(crate) title: String,
    /// `"May 2026"`.
    pub(crate) label: String,
    /// "1 to 31 May 2026, times in UTC. 14 services, 8 with a target."
    pub(crate) lede: String,
    pub(crate) generated: String,
    pub(crate) kpis: Kpis,
    pub(crate) groups: Vec<ReportGroup>,
    /// The months around this one (`2026-04`, label), when there is one.
    pub(crate) prev: Option<(String, String)>,
    pub(crate) next: Option<(String, String)>,
    /// The query of the month links: the viewer's token and theme, and the
    /// group of a scoped report.
    pub(crate) month_query: String,
}

/// The month at a glance.
pub(crate) struct Kpis {
    pub(crate) uptime: String,
    pub(crate) incidents: usize,
    pub(crate) incidents_note: String,
    pub(crate) downtime: String,
    pub(crate) mttr: String,
    /// "7 of 8"; empty without any target.
    pub(crate) targets: String,
    /// The services that missed their target, named.
    pub(crate) missed: String,
    pub(crate) all_met: bool,
}

pub(crate) struct ReportGroup {
    /// Display group name; empty for ungrouped monitors.
    name: String,
    /// Aggregate uptime across the group's monitors.
    uptime: String,
    rows: Vec<ReportRow>,
}

pub(crate) struct ReportRow {
    name: String,
    uptime: String,
    target: String,
    /// `up` (met), `down` (missed) or empty (no target or no data).
    met_state: &'static str,
    met_word: &'static str,
    incidents: String,
    downtime: String,
    mttr: String,
    left: String,
}

impl ReportGroup {
    /// How many services the group lists.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
}

/// Group the report rows (already in configuration order) and format them.
pub(crate) fn group_rows(
    report: &MonthReport,
    visible: impl Fn(&MonitorMonth) -> bool,
) -> Vec<ReportGroup> {
    let mut groups: Vec<(String, Vec<&MonitorMonth>)> = Vec::new();
    for row in report.rows.iter().filter(|row| visible(row)) {
        let key = row.group.clone().unwrap_or_default();
        match groups.iter_mut().find(|(name, _)| *name == key) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    // Ungrouped monitors render last, like on the status page.
    groups.sort_by_key(|(name, _)| name.is_empty());

    groups
        .into_iter()
        .map(|(name, rows)| ReportGroup {
            name,
            uptime: uptime_of(&rows),
            rows: rows.into_iter().map(format_row).collect(),
        })
        .collect()
}

/// The aggregate uptime of some rows: degraded checks count as available.
fn uptime_of(rows: &[&MonitorMonth]) -> String {
    let (available, total) = rows.iter().fold((0_i64, 0_i64), |(a, t), row| {
        (
            a + row.up + row.degraded,
            t + row.up + row.down + row.degraded,
        )
    });
    fmt::basis_points(available, total).map_or_else(|| "no data".to_owned(), pct_bp)
}

/// `99.97 %` (the page's spacing) from basis points.
fn pct_bp(basis_points: i64) -> String {
    fmt::pct_bp(basis_points).replace('%', " %")
}

/// The month at a glance, over the rows this viewer may see.
pub(crate) fn kpis(report: &MonthReport, visible: impl Fn(&MonitorMonth) -> bool) -> Kpis {
    let rows: Vec<&MonitorMonth> = report.rows.iter().filter(|row| visible(row)).collect();
    let incidents: usize = rows.iter().map(|row| row.incidents).sum();
    let downtime: i64 = rows.iter().map(|row| row.downtime_secs).sum();
    let mut mttrs: Vec<i64> = rows.iter().filter_map(|row| row.mttr_secs).collect();
    mttrs.sort_unstable();
    let targeted: Vec<&&MonitorMonth> = rows.iter().filter(|row| row.slo_met.is_some()).collect();
    let met = targeted
        .iter()
        .filter(|row| row.slo_met == Some(true))
        .count();
    let missed: Vec<&str> = targeted
        .iter()
        .filter(|row| row.slo_met == Some(false))
        .map(|row| row.name.as_str())
        .collect();
    Kpis {
        uptime: uptime_of(&rows),
        incidents,
        incidents_note: if incidents == 0 {
            "A quiet month".to_owned()
        } else {
            format!(
                "across {} service{}",
                rows.iter().filter(|row| row.incidents > 0).count(),
                if rows.iter().filter(|row| row.incidents > 0).count() == 1 {
                    ""
                } else {
                    "s"
                }
            )
        },
        downtime: minutes((downtime + 59) / 60),
        mttr: mttrs
            .get(mttrs.len() / 2)
            .map_or_else(|| "n/a".to_owned(), |secs| minutes((secs + 59) / 60)),
        targets: if targeted.is_empty() {
            String::new()
        } else {
            format!("{met} of {}", targeted.len())
        },
        all_met: missed.is_empty(),
        missed: if missed.is_empty() {
            String::new()
        } else {
            format!("{} missed", join_and(&missed))
        },
    }
}

/// A month link: its path segment (`2026-04`) and its name (`April`).
pub(crate) type MonthLink = Option<(String, String)>;

/// The month before and after `month` (`YYYY-MM`); the next one only once it
/// has started.
pub(crate) fn neighbours(month: &str) -> (MonthLink, MonthLink) {
    let Ok(first) = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d") else {
        return (None, None);
    };
    let prev = first.pred_opt().map(|day| {
        (
            day.format("%Y-%m").to_string(),
            day.format("%B").to_string(),
        )
    });
    let next = first
        .checked_add_months(chrono::Months::new(1))
        .filter(|day| *day <= Utc::now().date_naive())
        .map(|day| {
            (
                day.format("%Y-%m").to_string(),
                day.format("%B").to_string(),
            )
        });
    (prev, next)
}

/// "1 to 31 May 2026".
pub(crate) fn span(month: &str) -> String {
    let Ok(first) = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d") else {
        return month.to_owned();
    };
    let last = first
        .checked_add_months(chrono::Months::new(1))
        .and_then(|next| next.pred_opt())
        .map_or(28, |day| day.day());
    format!("1 to {last} {}", first.format("%B %Y"))
}

fn format_row(row: &MonitorMonth) -> ReportRow {
    let dash = || "\u{2014}".to_owned();
    let (met_state, met_word) = match row.slo_met {
        Some(true) => ("up", "Met"),
        Some(false) => ("down", "Missed"),
        None => ("", ""),
    };
    ReportRow {
        name: row.name.clone(),
        uptime: row.uptime_bp.map_or_else(|| "no data".to_owned(), pct_bp),
        target: row.slo_bp.map_or_else(dash, |slo| pct_bp(i64::from(slo))),
        met_state,
        met_word,
        incidents: row.incidents.to_string(),
        downtime: if row.downtime_secs > 0 {
            minutes((row.downtime_secs + 59) / 60)
        } else {
            "0 min".to_owned()
        },
        mttr: row
            .mttr_secs
            .map_or_else(|| "n/a".to_owned(), |secs| minutes((secs + 59) / 60)),
        left: match (row.budget_consumed_minutes, row.budget_minutes) {
            (Some(consumed), Some(budget)) => minutes((budget - consumed).max(0)),
            _ => dash(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_around_and_their_span() {
        let (prev, _) = neighbours("2026-03");
        assert_eq!(prev, Some(("2026-02".to_owned(), "February".to_owned())));
        // A month in the future has no "next".
        assert_eq!(neighbours("2999-01").1, None);
        assert_eq!(span("2026-02"), "1 to 28 February 2026");
        assert_eq!(span("2024-02"), "1 to 29 February 2024");
        assert_eq!(pct_bp(9997), "99.97 %");
    }
}
