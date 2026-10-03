//! Shared SQL fragments.
//!
//! Macros rather than consts so each query stays one `&'static str` literal
//! (`concat!` only takes literals), never a runtime-built string.

/// The `Incident` columns, in struct order.
macro_rules! incident_columns {
    () => {
        "id, monitor_id, started_at, ended_at, duration_s, cause, impacted, error, note, \
         snapshot, event, vantage, created_at"
    };
}

/// The `Announcement` columns.
macro_rules! announcement_columns {
    () => {
        "id, title, body, severity, until, created_at"
    };
}

/// The `Silence` columns.
macro_rules! silence_columns {
    () => {
        "id, monitor_id, until, reason, created_at"
    };
}

/// The `EventMarker` columns.
macro_rules! event_columns {
    () => {
        "id, title, created_at"
    };
}

/// `(available, total)` over `checks` rows: available = up or degraded.
macro_rules! available_total {
    () => {
        "CAST(COALESCE(SUM(CASE WHEN status IN (1, 2) THEN 1 ELSE 0 END), 0) AS INTEGER), \
         COUNT(*)"
    };
}

/// An upsert on a table keyed by `$key`: every listed column is written, and
/// all but the key are refreshed on conflict. Binds follow the column order.
macro_rules! upsert_sql {
    ($table:literal, $key:literal, $first:literal $(, $rest:literal)*) => {
        concat!(
            "INSERT INTO ", $table, " (", $key, ", ", $first, $(", ", $rest,)* ") ",
            "VALUES (?, ?", $(upsert_sql!(@param $rest),)* ") ",
            "ON CONFLICT(", $key, ") DO UPDATE SET ",
            $first, " = excluded.", $first,
            $(", ", $rest, " = excluded.", $rest,)*
        )
    };
    (@param $column:literal) => {
        ", ?"
    };
}
