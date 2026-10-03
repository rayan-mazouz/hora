//! Status ranking, labels and the daily uptime bar, shared by the summary and
//! the other views.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};

use hora_core::db::DayRow;
use hora_core::status::MonitorState;

use super::view::DayCell;

/// Whether the measured p95 meets the configured latency objective.
pub(crate) fn slo_state(target: Option<i64>, p95: Option<i64>) -> &'static str {
    match (target, p95) {
        (Some(target), Some(p95)) if p95 <= target => "met",
        (Some(_), Some(_)) => "breached",
        _ => "none",
    }
}

pub(crate) fn worse(current: MonitorState, candidate: MonitorState) -> MonitorState {
    if rank(candidate) > rank(current) {
        candidate
    } else {
        current
    }
}

/// How bad a state is for the overall badge: no data ranks above up (the
/// page cannot claim all is well) and below any trouble.
pub(crate) fn rank(status: MonitorState) -> u8 {
    match status {
        MonitorState::Up => 0,
        MonitorState::Unknown => 1,
        MonitorState::Degraded => 2,
        MonitorState::Down => 3,
    }
}

pub(crate) fn overall_label(status: MonitorState) -> &'static str {
    match status {
        MonitorState::Up => "All systems operational",
        MonitorState::Degraded => "Degraded performance",
        MonitorState::Down => "Major outage",
        MonitorState::Unknown => "Awaiting data",
    }
}

pub(crate) fn cert_state_for(days: Option<i64>, threshold: i64) -> &'static str {
    match days {
        None => "none",
        Some(remaining) if remaining <= 0 => "expired",
        Some(remaining) if remaining <= threshold => "warn",
        Some(_) => "ok",
    }
}

pub(crate) fn cert_label(days: i64) -> String {
    if days <= 0 {
        "expired".to_owned()
    } else {
        format!("{days}d")
    }
}

pub(crate) fn iso(timestamp: i64) -> Option<String> {
    DateTime::from_timestamp(timestamp, 0).map(|dt| dt.to_rfc3339())
}

/// Build the daily uptime bar (oldest to newest), zero-filling missing days.
pub(crate) fn build_bar(daily: &[DayRow], now: DateTime<Utc>, days: u16) -> Vec<DayCell> {
    let by_day: HashMap<&str, &DayRow> = daily.iter().map(|row| (row.day.as_str(), row)).collect();
    let mut cells = Vec::with_capacity(usize::from(days));

    for offset in (0..days).rev() {
        let date = (now - TimeDelta::days(i64::from(offset)))
            .format("%Y-%m-%d")
            .to_string();
        let cell = by_day.get(date.as_str()).map_or_else(
            || DayCell {
                title: format!("{date}: no data"),
                date: date.clone(),
                state: "empty",
                maint: false,
            },
            |row| day_cell(date.clone(), row),
        );
        cells.push(cell);
    }
    cells
}

/// A day stays amber as long as it was mostly up; it only turns red once the day
/// was a real outage (majority down).
pub(crate) const DAY_OUTAGE_BELOW_PERMILLE: i64 = 900; // < 90% availability over the day

pub(crate) fn day_cell(date: String, row: &DayRow) -> DayCell {
    let total = row.up + row.down + row.degraded;
    if total == 0 {
        let title = format!("{date}: no data");
        return DayCell {
            date,
            state: "empty",
            title,
            maint: false,
        };
    }
    let permille = (row.up + row.degraded).saturating_mul(1000) / total;
    let state = if row.down == 0 && row.degraded == 0 {
        "up"
    } else if permille >= DAY_OUTAGE_BELOW_PERMILLE {
        "degraded"
    } else {
        "down"
    };
    let title = format!("{date}: {}", hora_core::fmt::permille(permille));
    DayCell {
        date,
        state,
        title,
        maint: false,
    }
}
