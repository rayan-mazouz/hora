//! Operator writes: announcements, event markers and silences.

use axum::Json;
use axum::extract::{Query, State};
use chrono::Utc;
use serde::Deserialize;

use hora_core::db::{self};
use hora_core::notifications::{AlertSeverity, Event};

use crate::AppState;
use crate::auth::Admin;
use crate::error::AppError;
use hora_core::MAX_EVENT_TITLE_CHARS;
use hora_core::announce::{self, Announcement};
use hora_core::silence::{self, SilenceError};

#[derive(Debug, Deserialize)]
pub(crate) struct AnnounceQuery {
    /// Banner title (required, bounded).
    title: String,
    #[serde(default)]
    body: Option<String>,
    /// `info` (default) | `warning` | `critical` | `resolved`.
    #[serde(default)]
    severity: Option<String>,
    /// Auto-expiry: a duration (`4h`, `90m`) or the next occurrence of a UTC
    /// time (`18:00`), like `hora announce --until`; absent = until cleared.
    #[serde(default)]
    until: Option<String>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct AnnounceResponse {
    pub(crate) id: i64,
    /// When the banner auto-expires (unix epoch seconds), if bounded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) until: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/api/announce",
    params(
        ("title" = String, Query, description = "Banner title"),
        ("body" = Option<String>, Query, description = "Banner body"),
        ("severity" = Option<String>, Query, description = "info (default), warning, critical or resolved"),
        ("until" = Option<String>, Query, description = "Auto-expiry: a duration (e.g. 4h) or the next occurrence of a UTC time (HH:MM, e.g. 18:00)")
    ),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Announcement pinned to the status page", body = AnnounceResponse),
        (status = 400, description = "Empty title, unknown severity or unparseable duration"),
        (status = 401, description = "Missing or wrong token, or no admin_token configured")
    )
)]
pub(crate) async fn announce(
    State(state): State<AppState>,
    // Publishing to every visitor is an operator action, like /api/silence.
    _admin: Admin,
    Query(query): Query<AnnounceQuery>,
) -> Result<Json<AnnounceResponse>, AppError> {
    let severity = match query.severity.as_deref() {
        None => announce::Severity::Info,
        Some(value) => {
            announce::parse_severity(value).map_err(|err| AppError::BadRequest(err.message()))?
        }
    };
    let until = match query.until.as_deref() {
        None => None,
        Some(raw) => Some(announce::parse_until(raw, Utc::now().timestamp()).ok_or(
            AppError::BadRequest(announce::AnnounceError::InvalidUntil.message()),
        )?),
    };
    let announcement = Announcement::new(
        &query.title,
        query.body.as_deref().unwrap_or(""),
        severity,
        until,
    )
    .map_err(|err| AppError::BadRequest(err.message()))?;
    let id = announcement.pin(&state.store).await?;
    // Visitors should see the banner on their next view, not a refresh later.
    state.refresh_now().await;
    tracing::info!(
        title = %announcement.title,
        severity = %announcement.severity,
        "announcement pinned via API"
    );
    spawn_notice(
        &state,
        AlertSeverity::Info,
        format!("announcement pinned: {}", announcement.title),
    );
    Ok(Json(AnnounceResponse { id, until }))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct AnnounceClearResponse {
    /// How many still-pinned announcements were removed.
    pub(crate) cleared: u64,
}

#[utoipa::path(
    delete,
    path = "/api/announce",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every ad-hoc announcement removed", body = AnnounceClearResponse),
        (status = 401, description = "Missing or wrong token, or no admin_token configured")
    )
)]
pub(crate) async fn announce_clear(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<AnnounceClearResponse>, AppError> {
    let cleared = db::clear_announcements(&state.store, Utc::now().timestamp()).await?;
    state.refresh_now().await;
    tracing::info!(cleared, "announcements cleared via API");
    Ok(Json(AnnounceClearResponse { cleared }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventQuery {
    /// The event's title (required, bounded), e.g. `deploy api v2.3`.
    title: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct EventResponse {
    pub(crate) id: i64,
}

#[utoipa::path(
    post,
    path = "/api/event",
    params(
        ("title" = String, Query, description = "Event title, e.g. `deploy api v2.3`")
    ),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Event marker recorded", body = EventResponse),
        (status = 400, description = "Empty title"),
        (status = 401, description = "Missing or wrong token, or no admin_token configured")
    )
)]
pub(crate) async fn post_event(
    State(state): State<AppState>,
    // Recording a change marker is an operator action, same gesture as a
    // silence.
    _admin: Admin,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventResponse>, AppError> {
    let title = hora_core::bounded(&query.title, MAX_EVENT_TITLE_CHARS);
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty"));
    }
    let id = db::insert_event(&state.store, &title).await?;
    // The marker should appear on the operator's sparklines on the next view,
    // not a refresh later.
    state.refresh_now().await;
    tracing::info!(%title, "event marker recorded via API");
    Ok(Json(EventResponse { id }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct SilenceQuery {
    /// Comma-separated monitor ids, or `all` (stored as the `*` wildcard).
    monitors: String,
    /// How long to mute, e.g. `10m`, `1h30m`. Capped at 7 days.
    duration: String,
    #[serde(default)]
    reason: Option<String>,
    /// Allow silencing every monitor for longer than 24 hours.
    #[serde(default)]
    force: bool,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct SilenceResponse {
    /// The silenced monitor ids (`["*"]` for all).
    monitors: Vec<String>,
    /// When the silence expires (unix epoch seconds, UTC).
    until: i64,
}

#[utoipa::path(
    post,
    path = "/api/silence",
    params(
        ("monitors" = String, Query, description = "Comma-separated monitor ids, or `all`"),
        ("duration" = String, Query, description = "How long to mute (e.g. 10m, 1h30m; max 7d, and 24h for `all` without `force`)"),
        ("reason" = Option<String>, Query, description = "Optional note recorded with the silence"),
        ("force" = Option<bool>, Query, description = "Allow silencing `all` for longer than 24h")
    ),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Alerts muted until the returned time", body = SilenceResponse),
        (status = 400, description = "Unparseable duration, empty monitor list, or `all` past 24h without `force`"),
        (status = 401, description = "Missing or wrong token, or no admin_token configured"),
        (status = 404, description = "Unknown monitor id")
    )
)]
pub(crate) async fn silence(
    State(state): State<AppState>,
    // Muting alerts is an operator action: it strictly requires the
    // configured admin token.
    Admin { config }: Admin,
    Query(query): Query<SilenceQuery>,
) -> Result<Json<SilenceResponse>, AppError> {
    let silenced = silence::apply(
        &state.store,
        &config,
        &query.monitors,
        &query.duration,
        query.reason.as_deref(),
        query.force,
    )
    .await
    .map_err(|err| match err {
        SilenceError::InvalidDuration => {
            AppError::BadRequest("invalid duration (use e.g. 10m, 1h30m; max 7d)")
        }
        SilenceError::BlanketTooLong => AppError::BadRequest(
            "silencing all monitors is capped at 24h (add force=true to go up to 7d)",
        ),
        SilenceError::NoIds => AppError::BadRequest("no monitor ids given"),
        SilenceError::UnknownId(_) => AppError::NotFound("unknown monitor id"),
        SilenceError::Database(err) => AppError::Internal(err.into()),
    })?;
    tracing::info!(monitors = ?silenced.ids, until = silenced.until, "alerts silenced via API");
    spawn_notice(&state, AlertSeverity::Warning, silenced.notice());
    let (monitors, until) = (silenced.ids, silenced.until);
    Ok(Json(SilenceResponse { monitors, until }))
}

/// Tell every channel about an operator write made through the API, so a
/// leaked admin token cannot mute alerting or pin a banner unnoticed. Sent
/// straight to the channels: a silence never mutes it.
fn spawn_notice(state: &AppState, severity: AlertSeverity, title: String) {
    let notifier = state.notifier.clone();
    tokio::spawn(async move {
        notifier
            .load_full()
            .dispatch(
                Event::Alert {
                    monitor: "Hora",
                    severity,
                    title: &title,
                    message: "set through the API",
                },
                None,
            )
            .await;
    });
}
