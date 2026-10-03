//! HTTP endpoint handlers and their request/response types.

use std::future::Future;
use std::sync::{Arc, LazyLock};

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use badgelib::Style;
use chrono::Utc;
use serde::Deserialize;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};

use hora_core::config::{Config, Kind, Monitor};
use hora_core::db::{self, Point};
use hora_core::notifications::{AlertSeverity, Event};
use hora_core::peer::{HealthReport, PeerSeen};

use crate::auth::{Operator, QueryTokenAuth, Viewer, authorize_peer, ct_eq, push_token};
use crate::error::AppError;
use crate::flood;
use crate::history;
use crate::metrics;
use crate::render::{badge, status_color, svg_response, uptime_color};
use crate::summary::{
    DayCell, IncidentView, MaintenanceView, MonitorView, StatusTemplate, Summary,
};
use crate::text;
use crate::visibility::{Audience, Visibility};
use crate::{
    AppState, FAVICON_SVG, FONT_WOFF2, MAX_ALERT_DEDUP_CHARS, MAX_ALERT_TAG_CHARS, MAX_ALERT_TAGS,
    MAX_LATENCY_HOURS, MAX_LATENCY_POINTS, SECONDS_PER_HOUR, summary_for,
};
use hora_core::announce::{self, Announcement};
use hora_core::silence::{self, SilenceError};
use hora_core::{MAX_ALERT_TITLE_CHARS, MAX_EVENT_TITLE_CHARS, MAX_PUSH_MSG_CHARS};

/// The `OpenAPI` document, generated once at startup (empty if generation fails).
pub(crate) static OPENAPI_JSON: LazyLock<String> = LazyLock::new(|| {
    ApiDoc::openapi().to_pretty_json().unwrap_or_else(|err| {
        tracing::error!("failed to generate OpenAPI document: {err}");
        String::new()
    })
});

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Hora API",
        description = "JSON API of a Hora uptime monitor: read-only status views, plus token-gated writes (heartbeats, alerts, silences, announcements, events) and the peer-mesh endpoints."
    ),
    modifiers(&SecuritySchemes),
    paths(
        summary_json,
        latency_json,
        push,
        post_alert,
        silence,
        announce,
        announce_clear,
        post_event,
        peer_probe,
        peer_monitors,
        status_badge,
        uptime_badge,
        heatmap_svg,
        healthz
    ),
    components(schemas(
        Summary,
        MonitorView,
        IncidentView,
        MaintenanceView,
        DayCell,
        Point,
        HealthReport,
        PeerSeen,
        SilenceResponse,
        AnnounceResponse,
        AnnounceClearResponse,
        AlertRequest,
        AlertResponse,
        EventResponse,
        hora_core::confirm::ProbeRequest,
        hora_core::confirm::ProbeResponse,
        hora_core::confirm::PeerMonitors,
        hora_core::confirm::PeerMonitor,
        crate::summary::VantageView
    ))
)]
struct ApiDoc;

/// The two credentials the API understands: the viewer/operator token as
/// `Authorization: Bearer`, and an item token (push monitor, alerting
/// monitor, mesh peer) as `X-Push-Token`.
struct SecuritySchemes;

impl utoipa::Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
        components.add_security_scheme(
            "push_token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new(
                crate::auth::PUSH_TOKEN_HEADER,
            ))),
        );
    }
}

#[utoipa::path(
    get,
    path = "/healthz",
    responses(
        (status = 200, description = "Node healthy; its view of watched peers", body = HealthReport),
        (status = 503, description = "Scheduler stalled or database unreachable (same body)", body = HealthReport)
    )
)]
pub(crate) async fn healthz(State(state): State<AppState>) -> Response {
    let config = state.config.borrow().clone();
    let report = hora_core::peer::report(&state.pool, &config, &state.last_tick).await;
    // A degraded node answers 503 so plain health checks (Docker's
    // HEALTHCHECK, a load balancer) notice; the body is unchanged for the
    // peers and keyword monitors that read it.
    let status = if report.status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(report)).into_response()
}

pub(crate) async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
    )
}

pub(crate) async fn font() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        FONT_WOFF2,
    )
}

pub(crate) async fn openapi() -> Response {
    if OPENAPI_JSON.is_empty() {
        // The document is static; an empty one means generation failed at startup.
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "OpenAPI generation failed",
        )
            .into_response();
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        OPENAPI_JSON.as_str(),
    )
        .into_response()
}

/// Fetch (or build) the cached summary for `audience`. Infallible: a failing
/// monitor degrades to an `unknown` card rather than failing the page.
pub(crate) async fn state_summary(
    state: &AppState,
    config: &Arc<Config>,
    audience: &Audience,
) -> Arc<Summary> {
    summary_for(
        &state.pool,
        config,
        &state.cache,
        audience,
        &state.notifier,
        &state.vantage,
    )
    .await
}

/// Text clients (curl, wget, or an explicit text/plain Accept) get the aligned
/// plain-text rendering; everyone else the HTML page.
fn wants_text(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ua| ua.starts_with("curl/") || ua.starts_with("Wget/"))
        || headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/plain") && !accept.contains("text/html"))
}

/// Render a status summary as the HTML page or, for text clients, plain text.
fn status_response(headers: &HeaderMap, summary: &Summary) -> Result<Response, AppError> {
    if wants_text(headers) {
        let body = text::render(summary);
        Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response())
    } else {
        let html = StatusTemplate { summary }.render()?;
        Ok(Html(html).into_response())
    }
}

pub(crate) async fn page(
    State(state): State<AppState>,
    viewer: Viewer,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let summary = state_summary(&state, &viewer.config, &viewer.audience).await;
    status_response(&headers, &summary)
}

/// The per-group status page (`/status/{group}`): the monitors of one display
/// group, nothing else - lightweight multi-tenancy for an operator hosting
/// several clients' services on one Hora. Anonymous viewers get the group's
/// public monitors; this group's own `server.group_tokens` entry reveals the
/// group's full view (private monitors, full failure detail) but never the
/// operator's streams (deploy markers) nor another group's private names; the
/// global viewer token is the operator. An unknown group - or a fully private
/// one viewed without a token - answers 404, exactly like a missing page.
pub(crate) async fn group_page(
    State(state): State<AppState>,
    Path(group): Path<String>,
    viewer: Viewer,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let audience = viewer.audience_for_group(&group);
    let summary = state_summary(&state, &viewer.config, &audience).await;
    let view = crate::summary::for_group(&summary, &viewer.config, &group)
        .ok_or(AppError::NotFound("unknown group"))?;
    status_response(&headers, &view)
}

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
        ("until" = Option<String>, Query, description = "Auto-expiry: a duration (e.g. 4h) or the next occurrence of a UTC time (HH:MM, e.g. 18:00)"),
        ("token" = Option<String>, Query, deprecated, description = "Viewer token; deprecated on this endpoint (answered with a Deprecation header): use Authorization: Bearer")
    ),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Announcement pinned to the status page", body = AnnounceResponse),
        (status = 400, description = "Empty title, unknown severity or unparseable duration"),
        (status = 401, description = "Missing or wrong token, or no auth_token configured")
    )
)]
pub(crate) async fn announce(
    State(state): State<AppState>,
    // Publishing to every visitor is an operator action, like /api/silence.
    _operator: Operator,
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
    let id = announcement.pin(&state.pool).await?;
    // Visitors should see the banner now, not when the summary cache rolls.
    state.cache.invalidate();
    tracing::info!(
        title = %announcement.title,
        severity = %announcement.severity,
        "announcement pinned via API"
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
    params(("token" = Option<String>, Query, deprecated, description = "Viewer token; deprecated on this endpoint (answered with a Deprecation header): use Authorization: Bearer")),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every ad-hoc announcement removed", body = AnnounceClearResponse),
        (status = 401, description = "Missing or wrong token, or no auth_token configured")
    )
)]
pub(crate) async fn announce_clear(
    State(state): State<AppState>,
    _operator: Operator,
) -> Result<Json<AnnounceClearResponse>, AppError> {
    let cleared = db::clear_announcements(&state.pool, Utc::now().timestamp()).await?;
    state.cache.invalidate();
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
        ("title" = String, Query, description = "Event title, e.g. `deploy api v2.3`"),
        ("token" = Option<String>, Query, deprecated, description = "Viewer token; deprecated on this endpoint (answered with a Deprecation header): use Authorization: Bearer")
    ),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Event marker recorded", body = EventResponse),
        (status = 400, description = "Empty title"),
        (status = 401, description = "Missing or wrong token, or no auth_token configured")
    )
)]
pub(crate) async fn post_event(
    State(state): State<AppState>,
    // Recording a change marker is an operator action, same gesture as a
    // silence.
    _operator: Operator,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventResponse>, AppError> {
    let title = hora_core::bounded(&query.title, MAX_EVENT_TITLE_CHARS);
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty"));
    }
    let id = db::insert_event(&state.pool, &title).await?;
    // The marker should appear on the operator's sparklines now, not when the
    // summary cache rolls.
    state.cache.invalidate();
    tracing::info!(%title, "event marker recorded via API");
    Ok(Json(EventResponse { id }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReportQuery {
    /// Restrict the report to one display group - the report twin of the
    /// `/status/{group}` page, and what an operator hands a client.
    #[serde(default)]
    group: Option<String>,
}

/// The printable monthly SLA report (`/report/2026-05`, optionally
/// `?group=X`). Anonymous viewers get the public monitors; the viewer token
/// includes the private ones, and on a `?group=` report the group's own
/// `server.group_tokens` entry does too - so a client can be handed *their*
/// report and nothing else. Server-rendered and print-first: "Save as PDF"
/// is the export.
pub(crate) async fn report_page(
    State(state): State<AppState>,
    Path(month): Path<String>,
    viewer: Viewer,
    Query(query): Query<ReportQuery>,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    // Validate the month *before* building, so a malformed path is a clean
    // 400 and never reaches the database.
    let Some((_, end)) = hora_core::report::month_bounds(&month) else {
        return Err(AppError::BadRequest(
            "month must be YYYY-MM and not in the future",
        ));
    };
    // How far back a monthly report may reach: the daily aggregates it reads
    // live that long; an older month would only be an empty - and needlessly
    // expensive - report.
    if end <= Utc::now().timestamp() - db::AGGREGATE_RETENTION_DAYS * hora_core::SECONDS_PER_DAY {
        return Err(AppError::BadRequest(
            "month is older than the retained history (12 months)",
        ));
    }
    let audience = match query.group.as_deref() {
        Some(group) => viewer.audience_for_group(group),
        None => viewer.audience.clone(),
    };
    let visibility = Visibility::new(config, &audience);

    // The built report does not depend on the audience (rows are filtered
    // below), so one copy per month serves every viewer for a minute.
    let report = if let Some(report) = state.reports.get(&month, config) {
        report
    } else {
        let report = Arc::new(hora_core::report::build(&state.pool, config, &month).await?);
        state.reports.insert(&month, config, Arc::clone(&report));
        report
    };
    let groups = crate::report::group_rows(&report, |row| {
        visibility.can_see(&row.id)
            && query
                .group
                .as_deref()
                .is_none_or(|group| row.group.as_deref() == Some(group))
    });
    // A scoped report with nothing visible answers like the group page: 404,
    // revealing neither the group's existence nor its members.
    if groups.is_empty() && query.group.is_some() {
        return Err(AppError::NotFound("unknown group"));
    }
    let title = match &query.group {
        Some(group) => format!("{} · {group}", config.page.title),
        None => config.page.title.clone(),
    };
    let html = crate::report::ReportTemplate {
        title,
        label: report.label.clone(),
        generated: Utc::now().format("%Y-%m-%d %H:%M UTC").to_string(),
        groups,
    }
    .render()?;
    Ok(Html(html))
}

#[utoipa::path(
    get,
    path = "/api/summary",
    security((), ("bearer" = [])),
    responses((status = 200, description = "Status of every monitor", body = Summary))
)]
pub(crate) async fn summary_json(
    State(state): State<AppState>,
    viewer: Viewer,
) -> Json<Arc<Summary>> {
    Json(state_summary(&state, &viewer.config, &viewer.audience).await)
}

pub(crate) async fn metrics_prometheus(
    State(state): State<AppState>,
    viewer: Viewer,
) -> impl IntoResponse {
    let summary = state_summary(&state, &viewer.config, &viewer.audience).await;
    let body = metrics::render(&summary);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

/// The most rows a visibility-filtered list scans to fill its page: the
/// newest rows may all belong to private monitors, so the scan widens until
/// `limit` visible rows are found - bounded, since the endpoints are public.
const MAX_VISIBILITY_SCAN: i64 = 5_000;

/// Collect up to `limit` rows the audience may see, filtering *before*
/// truncating: `fetch(n)` returns the newest `n` rows, `filter` keeps (and
/// sanitizes) the visible ones. The window widens 4x per round until enough
/// rows are visible, the table is exhausted, or [`MAX_VISIBILITY_SCAN`] is hit.
async fn collect_visible<T, Fut>(
    limit: i64,
    mut fetch: impl FnMut(i64) -> Fut,
    filter: impl Fn(&mut Vec<T>),
) -> Result<Vec<T>, AppError>
where
    Fut: Future<Output = sqlx::Result<Vec<T>>>,
{
    let wanted = usize::try_from(limit).unwrap_or(0);
    let mut window = limit.max(1);
    loop {
        let mut rows = fetch(window).await?;
        let exhausted = usize::try_from(window).is_ok_and(|window| rows.len() < window);
        filter(&mut rows);
        if rows.len() >= wanted || exhausted || window >= MAX_VISIBILITY_SCAN {
            rows.truncate(wanted);
            return Ok(rows);
        }
        window = window.saturating_mul(4).min(MAX_VISIBILITY_SCAN);
    }
}

/// Recent incidents restricted to (and sanitized for) what the audience may
/// see - see [`Visibility::sanitize_incident`].
async fn visible_incidents(
    pool: &sqlx::SqlitePool,
    visibility: &Visibility<'_>,
    limit: i64,
) -> Result<Vec<db::Incident>, AppError> {
    collect_visible(
        limit,
        |window| db::recent_incidents(pool, window),
        |rows| visibility.filter_incidents(rows),
    )
    .await
}

/// Recent pushed alerts restricted to (and sanitized for) what the audience
/// may see - see [`Visibility::sanitize_alert`].
async fn visible_pushed_alerts(
    pool: &sqlx::SqlitePool,
    visibility: &Visibility<'_>,
    limit: i64,
) -> Result<Vec<db::PushedAlert>, AppError> {
    collect_visible(
        limit,
        |window| db::recent_pushed_alerts(pool, window),
        |rows| visibility.filter_alerts(rows),
    )
    .await
}

/// The auto-generated post-mortem page (`/incident/{id}`): everything the
/// incident already knows, assembled server-side, with the raw markdown ready
/// to copy into a ticket. Visibility mirrors the history page: a private
/// monitor's incident (or one whose monitor left the config) answers 404 to
/// anonymous viewers, and a public one is sanitized unless the monitor opted
/// into `public_error_detail`.
pub(crate) async fn incident_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    viewer: Viewer,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let mut incident = db::incident_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound("unknown incident"))?;
    if !visibility.can_see(&incident.monitor_id) {
        return Err(AppError::NotFound("unknown incident"));
    }
    visibility.sanitize_incident(&mut incident);

    let monitor_name = config.monitor_name(&incident.monitor_id).to_owned();
    let markdown = hora_core::postmortem::render(&incident, &monitor_name);
    let html = crate::history::IncidentTemplate {
        title: config.page.title.clone(),
        monitor: monitor_name,
        row: crate::history::incident_rows(std::slice::from_ref(&incident), &monitor_names(config))
            .remove(0),
        markdown,
    }
    .render()?;
    Ok(Html(html))
}

/// Map of monitor id to display name, for rendering incidents.
fn monitor_names(config: &Config) -> std::collections::HashMap<String, String> {
    config
        .monitors
        .iter()
        .map(|monitor| (monitor.id.clone(), monitor.name.clone()))
        .collect()
}

pub(crate) async fn history_page(
    State(state): State<AppState>,
    viewer: Viewer,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let incidents = visible_incidents(&state.pool, &visibility, 100).await?;
    let pushed_alerts = visible_pushed_alerts(&state.pool, &visibility, 100).await?;
    // Event markers are operator info (deploy titles).
    let events = if visibility.sees_operator_streams() {
        db::recent_events(&state.pool, 100).await?
    } else {
        Vec::new()
    };
    // The heatmap section lists what this viewer may see; the images load
    // lazily from the API. Push monitors have no latency series to show.
    let heatmaps = config
        .monitors
        .iter()
        .filter(|monitor| visibility.can_see_monitor(monitor) && monitor.kind != Kind::Push)
        .map(|monitor| history::HeatmapRef {
            id: monitor.id.clone(),
            name: monitor.name.clone(),
        })
        .collect();
    let names = monitor_names(config);
    let html = history::HistoryTemplate {
        title: config.page.title.clone(),
        incidents: history::incident_rows(&incidents, &names),
        pushed_alerts: history::alert_rows(&pushed_alerts, &names),
        events: history::event_rows(&events),
        heatmaps,
        token_query: viewer.token_query.clone(),
    }
    .render()?;
    Ok(Html(html))
}

/// How far back the `/timeline` page reaches, and how many entries it shows.
const TIMELINE_DAYS: i64 = 7;
const TIMELINE_LIMIT: usize = 200;

/// The unified chronology (`/timeline`): downs/recoveries, operator events,
/// pushed alerts, announcements and silences merged newest-first. The
/// operator view gets everything; anyone else gets the same subset they may
/// see elsewhere - sanitized visible incidents and announcements - and never
/// the operator streams (events, silences, alert detail).
pub(crate) async fn timeline_page(
    State(state): State<AppState>,
    viewer: Viewer,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let since = Utc::now().timestamp() - TIMELINE_DAYS * hora_core::SECONDS_PER_DAY;

    let sources = if visibility.sees_operator_streams() {
        hora_core::timeline::fetch(&state.pool, since, 500).await?
    } else {
        let mut incidents = visible_incidents(&state.pool, &visibility, 500).await?;
        incidents.retain(|incident| {
            incident.started_at >= since || incident.ended_at.is_some_and(|ended| ended >= since)
        });
        hora_core::timeline::Sources {
            incidents,
            announcements: db::announcements_since(&state.pool, since).await?,
            ..Default::default()
        }
    };
    let entries =
        hora_core::timeline::merge(&sources, &monitor_names(config), since, TIMELINE_LIMIT);

    let html = history::TimelineTemplate {
        title: config.page.title.clone(),
        entries: history::timeline_rows(&entries),
        token_query: viewer.token_query.clone(),
    }
    .render()?;
    Ok(Html(html))
}

pub(crate) async fn history_atom(
    State(state): State<AppState>,
    viewer: Viewer,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let incidents = visible_incidents(&state.pool, &visibility, 50).await?;
    // Absolute feed links: scheme from the proxy's x-forwarded-proto (plain
    // http when absent), host from the Host header.
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|proto| *proto == "https" || *proto == "http")
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let base_url = format!("{proto}://{host}");
    let body = history::render_atom(
        &incidents,
        &monitor_names(config),
        &base_url,
        &config.page.title,
    );
    Ok((
        [(header::CONTENT_TYPE, "application/atom+xml; charset=utf-8")],
        body,
    ))
}

/// The configured monitor `id`, if this audience may see it. A private
/// monitor answers exactly like a missing one (404) - its existence is not
/// revealed either way.
fn visible_monitor<'a>(
    config: &'a Config,
    visibility: &Visibility<'_>,
    id: &str,
) -> Result<&'a Monitor, AppError> {
    config
        .monitors
        .iter()
        .find(|monitor| monitor.id == id && visibility.can_see_monitor(monitor))
        .ok_or(AppError::NotFound("unknown monitor"))
}

#[derive(Debug, Deserialize)]
pub(crate) struct LatencyQuery {
    #[serde(default = "default_hours")]
    hours: i64,
}

pub(crate) fn default_hours() -> i64 {
    24
}

#[utoipa::path(
    get,
    path = "/api/monitors/{id}/latency",
    params(
        ("id" = String, Path, description = "Monitor id"),
        ("hours" = Option<i64>, Query, description = "Look-back window in hours (1..=720)")
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "Latency samples, oldest first", body = [Point]),
        (status = 404, description = "Unknown monitor")
    )
)]
pub(crate) async fn latency_json(
    State(state): State<AppState>,
    Path(id): Path<String>,
    viewer: Viewer,
    Query(query): Query<LatencyQuery>,
) -> Result<Json<Vec<Point>>, AppError> {
    let visibility = Visibility::new(&viewer.config, &viewer.audience);
    visible_monitor(&viewer.config, &visibility, &id)?;
    let window = query.hours.clamp(1, MAX_LATENCY_HOURS) * SECONDS_PER_HOUR;
    let since = Utc::now().timestamp() - window;
    // Average into at most MAX_LATENCY_POINTS buckets in SQL, so a 10s-interval
    // monitor over 720h (~260k raw rows) never materializes more than the cap.
    // Ceiling division keeps the bucket count under the cap even for short
    // windows, where flooring would produce up to ~2x the buckets.
    let max_points = i64::try_from(MAX_LATENCY_POINTS).expect("MAX_LATENCY_POINTS fits in i64");
    // (Manual ceil: `i64::div_ceil` is still unstable.)
    let bucket_secs = ((window + max_points - 1) / max_points).max(1);
    let points = db::latency_series(&state.pool, &id, since, bucket_secs).await?;
    // The SQL already respects the cap; downsample stays as a pure backstop.
    Ok(Json(downsample(points, MAX_LATENCY_POINTS)))
}

#[utoipa::path(
    post,
    path = "/api/peer/probe",
    request_body = hora_core::confirm::ProbeRequest,
    security(("push_token" = [])),
    responses(
        (status = 200, description = "This vantage's verdict on the target", body = hora_core::confirm::ProbeResponse),
        (status = 400, description = "Exec and push monitors are not network probes"),
        (status = 401, description = "Unknown requesting peer, or missing/wrong X-Push-Token"),
        (status = 404, description = "The target is not in this node's configuration")
    )
)]
pub(crate) async fn peer_probe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<hora_core::confirm::ProbeRequest>,
) -> Result<Json<hora_core::confirm::ProbeResponse>, AppError> {
    let config = state.config.borrow().clone();
    // Probing is strictly more sensitive than a push heartbeat: the peer must
    // be known here with a listen_token, and the header must match.
    authorize_peer(&config, &headers, &request.from)?;

    // Exec and push monitors have no network target: every exec monitor has
    // the same empty one, so matching it would run *some* local exec monitor
    // and answer its verdict (or a bogus "down") for another node's check.
    if matches!(request.kind, Kind::Exec | Kind::Push) {
        return Err(AppError::BadRequest(
            "exec and push monitors are not network probes",
        ));
    }

    // The SSRF guard: only targets present in THIS node's configuration are
    // probed - a peer is a vantage point, never a proxy. The matched
    // monitor's own settings (timeout, assertions, proxy) drive the probe.
    let monitor = config
        .monitors
        .iter()
        .find(|monitor| monitor.kind == request.kind && monitor.target == request.target)
        .ok_or(AppError::NotFound(
            "target not in this node's configuration",
        ))?;

    // Single attempt (no retries): the requester wants a fast vantage check,
    // not this node's anti-flap pipeline. The outer timeout is a backstop on
    // top of the probe's own; if even that elapses, the target is
    // unresponsive *from here*, which is a down verdict in its own right.
    let mut probe_monitor = monitor.clone();
    probe_monitor.probe_retries = Some(0);
    let client = state
        .probe_clients
        .get_or_build(
            &config,
            monitor.proxy.as_deref(),
            hora_core::http::probe_client,
        )
        .map_err(|err| AppError::Internal(err.into()))?;
    let outcome = match tokio::time::timeout(
        hora_core::confirm::PROBE_DEADLINE,
        hora_core::probe::run(&client, &probe_monitor),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_elapsed) => {
            return Ok(Json(hora_core::confirm::ProbeResponse {
                up: false,
                error: Some("probe timed out at this vantage".to_owned()),
            }));
        }
    };
    Ok(Json(hora_core::confirm::ProbeResponse {
        up: outcome.up,
        // Bounded: the reason crosses the wire into another node's logs.
        error: outcome.error.map(|error| error.chars().take(200).collect()),
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct PeerMonitorsQuery {
    /// The requesting node's `[health].id`.
    from: String,
}

#[utoipa::path(
    get,
    path = "/api/peer/monitors",
    params(("from" = String, Query, description = "The requesting peer's [health].id")),
    security(("push_token" = [])),
    responses(
        (status = 200, description = "This node's probeable monitors, with its own view of each (status, 24h median)", body = hora_core::confirm::PeerMonitors),
        (status = 401, description = "Unknown requesting peer, or missing/wrong X-Push-Token")
    )
)]
pub(crate) async fn peer_monitors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PeerMonitorsQuery>,
) -> Result<Json<hora_core::confirm::PeerMonitors>, AppError> {
    let config = state.config.borrow().clone();
    // Same strict authentication as /api/peer/probe.
    authorize_peer(&config, &headers, &query.from)?;

    // What a mesh member may know: kind + target (it can probe those anyway
    // via /api/peer/probe) and this node's live view of each - never names,
    // notes or credentials. Push/exec monitors have no probeable target.
    // Peers poll every minute, so this reads only the two inputs it answers
    // with (the summary's status and p50), never the full page rebuild.
    let shared: Vec<&Monitor> = config
        .monitors
        .iter()
        .filter(|monitor| !matches!(monitor.kind, Kind::Push | Kind::Exec))
        .collect();
    let threshold = i64::from(config.alerts.fail_threshold.max(1));
    let since_24h = Utc::now().timestamp() - hora_core::SECONDS_PER_DAY;
    let (recent, percentiles) = tokio::join!(
        crate::summary::recent_checks_map(&state.pool, &shared, threshold),
        db::latency_percentiles_all(&state.pool, since_24h),
    );
    let percentiles = crate::summary::or_empty(percentiles, "latency percentiles");
    let monitors = shared
        .iter()
        .map(|monitor| hora_core::confirm::PeerMonitor {
            kind: monitor.kind,
            target: monitor.target.clone(),
            status: recent.get(&monitor.id).map_or_else(
                || "unknown".to_owned(),
                |checks| db::derive_status(checks, threshold).to_owned(),
            ),
            p50_ms: percentiles.get(&monitor.id).map(|(p50, _, _)| *p50),
        })
        .collect();
    Ok(Json(hora_core::confirm::PeerMonitors { monitors }))
}

#[utoipa::path(
    get,
    path = "/api/monitors/{id}/heatmap.svg",
    params(("id" = String, Path, description = "Monitor id")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "28-day hours-by-days latency heatmap (SVG)"),
        (status = 404, description = "Unknown monitor")
    )
)]
pub(crate) async fn heatmap_svg(
    State(state): State<AppState>,
    Path(id): Path<String>,
    viewer: Viewer,
) -> Result<impl IntoResponse, AppError> {
    let config = &viewer.config;
    // Same visibility rule as the latency endpoint.
    let visibility = Visibility::new(config, &viewer.audience);
    let monitor = visible_monitor(config, &visibility, &id)?;
    // The image does not depend on who asks (visibility is settled above), and
    // it groups 28 days of checks: one render per monitor serves everyone for
    // a minute.
    if let Some(svg) = state.heatmaps.get(&id, config) {
        return Ok(svg_response(svg.as_ref().clone()));
    }
    let now = Utc::now().timestamp();
    let since = (now / 86_400 - (crate::heatmap::HEATMAP_DAYS - 1)) * 86_400;
    let cells = db::latency_hourly(&state.pool, &id, since).await?;
    let svg = crate::heatmap::render(&cells, now, &monitor.name);
    state.heatmaps.insert(&id, config, Arc::new(svg.clone()));
    Ok(svg_response(svg))
}

#[derive(Debug, Deserialize)]
pub(crate) struct PushQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    ping: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/api/push/{id}",
    params(
        ("id" = String, Path, description = "Push monitor id"),
        ("token" = Option<String>, Query, deprecated, description = "Push token, if the monitor sets one; deprecated (answered with a Deprecation header): use X-Push-Token"),
        ("status" = Option<String>, Query, description = "up (default), down or degraded"),
        ("msg" = Option<String>, Query, description = "Optional detail recorded with the heartbeat"),
        ("ping" = Option<i64>, Query, description = "Optional round-trip latency in ms (>= 0)")
    ),
    security((), ("push_token" = [])),
    responses(
        (status = 200, description = "Heartbeat recorded"),
        (status = 400, description = "Unknown status or negative ping"),
        (status = 401, description = "Missing or wrong token"),
        (status = 404, description = "Unknown push monitor")
    )
)]
pub(crate) async fn push(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query_auth: QueryTokenAuth,
    Query(query): Query<PushQuery>,
    headers: HeaderMap,
) -> Result<&'static str, AppError> {
    let config = state.config.borrow().clone();
    // A push id is either a push monitor or a watched peer's listen id (peers
    // heartbeat the same endpoint); the expected token comes from whichever matches.
    let expected_token = if let Some(monitor) = config
        .monitors
        .iter()
        .find(|monitor| monitor.id == id && monitor.kind == Kind::Push)
    {
        monitor.push_token.as_ref()
    } else if let Some(peer) = config
        .peers
        .iter()
        .find(|peer| peer.is_watched() && peer.listen_id() == id)
    {
        peer.listen_token.as_ref()
    } else {
        return Err(AppError::NotFound("unknown push target"));
    };

    // A configured token is required; without one, the id alone authorizes. Prefer
    // the `X-Push-Token` header (kept out of access logs) over the `?token=` query.
    if let Some(expected) = expected_token {
        let header = push_token(&headers);
        let provided = header.or(query.token.as_deref());
        if !provided.is_some_and(|token| ct_eq(token, expected.as_ref())) {
            return Err(AppError::Unauthorized("invalid push token"));
        }
        if header.is_none() {
            query_auth.mark();
        }
    }

    // A typo'd status must fail loudly: recording `status=dwon` as "up" would
    // report the opposite of what the job meant.
    let status = match query.status.as_deref() {
        None | Some("" | "up") => 1,
        Some("down") => 0,
        Some("degraded") => 2,
        Some(_) => {
            return Err(AppError::BadRequest("status must be up, down or degraded"));
        }
    };
    if query.ping.is_some_and(|ping| ping < 0) {
        return Err(AppError::BadRequest("ping must not be negative"));
    }
    // Bound the stored message so a buggy or hostile pusher can't bloat the DB.
    let msg = query
        .msg
        .as_deref()
        .map(|msg| msg.chars().take(MAX_PUSH_MSG_CHARS).collect::<String>());
    db::insert_push(&state.pool, &id, status, query.ping, msg.as_deref()).await?;
    Ok("ok")
}

/// The JSON body of `POST /api/monitors/{id}/alert`. Every field is optional at
/// the deserialization layer so a missing one yields a clean 400 from the
/// handler rather than a 422 from the extractor.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct AlertRequest {
    /// `info` (default) | `warning` | `error` | `critical`.
    #[serde(default)]
    severity: Option<String>,
    /// Short headline (required, bounded).
    #[serde(default)]
    title: Option<String>,
    /// Free-form detail (optional, bounded).
    #[serde(default)]
    message: Option<String>,
    /// Coalescing key for the server-side anti-flood window (optional).
    #[serde(default)]
    dedup_key: Option<String>,
    /// Arbitrary key/value pairs folded into the message (optional).
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct AlertResponse {
    /// `dispatched` (sent to the channels and recorded) or `coalesced` (a
    /// `dedup_key` repeat dropped inside the anti-flood window).
    status: &'static str,
    /// The accepted severity.
    severity: &'static str,
    /// The recorded alert's id - only when `dispatched`.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    /// The dedup key - echoed only when `coalesced`.
    #[serde(skip_serializing_if = "Option::is_none")]
    dedup_key: Option<String>,
    /// Repeats coalesced in this window so far - only when `coalesced`.
    #[serde(skip_serializing_if = "Option::is_none")]
    suppressed: Option<u64>,
    /// Seconds until the window reopens - only when `coalesced`.
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_secs: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/api/monitors/{id}/alert",
    params(
        ("id" = String, Path, description = "Monitor id the alert is attached to"),
        ("token" = Option<String>, Query, deprecated, description = "server.auth_token; deprecated (answered with a Deprecation header): use Authorization: Bearer or X-Push-Token")
    ),
    request_body = AlertRequest,
    security(("push_token" = []), ("bearer" = [])),
    responses(
        (status = 202, description = "Alert dispatched to the monitor's channels (or coalesced)", body = AlertResponse),
        (status = 400, description = "Empty title or unknown severity"),
        (status = 401, description = "Missing/wrong X-Push-Token and no matching server.auth_token (also for an unknown monitor)"),
        (status = 404, description = "Unknown monitor (operator token only)")
    )
)]
pub(crate) async fn post_alert(
    State(state): State<AppState>,
    Path(id): Path<String>,
    viewer: Viewer,
    headers: HeaderMap,
    Json(request): Json<AlertRequest>,
) -> Result<(StatusCode, Json<AlertResponse>), AppError> {
    let config = &viewer.config;
    let monitor = config.find_monitor(&id);

    // Authenticate with the monitor's own push_token (preferred, via the
    // X-Push-Token header kept out of access logs) or the global viewer token.
    // Dispatching to channels can flood, so - unlike a read-only view - the
    // endpoint stays closed unless a credential is configured and matches.
    // Without the operator token an unknown id answers the same 401 as a
    // known one, so the endpoint cannot be used to probe private monitor ids.
    let item_token_ok = monitor
        .and_then(|monitor| monitor.push_token.as_ref())
        .is_some_and(|expected| {
            push_token(&headers).is_some_and(|token| ct_eq(token, expected.as_ref()))
        });
    if !item_token_ok && !viewer.is_operator() {
        return Err(AppError::Unauthorized(
            "alerting requires the monitor's push_token (X-Push-Token) or server.auth_token",
        ));
    }
    if !item_token_ok {
        viewer.note_operator_write();
    }
    let monitor = monitor.ok_or(AppError::NotFound("unknown monitor"))?;

    // Validate the body.
    let severity = match request.severity.as_deref() {
        None | Some("") => AlertSeverity::Info,
        Some(value) => AlertSeverity::parse(value).ok_or(AppError::BadRequest(
            "severity must be info, warning, error or critical",
        ))?,
    };
    let title: String = request
        .title
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_ALERT_TITLE_CHARS)
        .collect();
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty"));
    }
    let dedup_key: Option<String> = request
        .dedup_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| key.chars().take(MAX_ALERT_DEDUP_CHARS).collect());
    // The producer's text, with any tags folded in ("just enrich the message"),
    // bounded so a buggy producer can't bloat the row.
    let message = render_alert_message(request.message.as_deref().unwrap_or(""), &request.tags);

    // Anti-flood: a dedup_key that recurs inside the window is coalesced here,
    // so the rate-limiting lives in Hora and every producer benefits.
    let window = i64::try_from(config.alerts.push_alert_window_secs).unwrap_or(i64::MAX);
    let now = Utc::now().timestamp();
    if let flood::Admission::Coalesce {
        suppressed,
        retry_after,
    } = state.flood.admit(&id, dedup_key.as_deref(), now, window)
    {
        tracing::info!(monitor = %id, suppressed, "pushed alert coalesced");
        return Ok((
            StatusCode::ACCEPTED,
            Json(AlertResponse {
                status: "coalesced",
                severity: severity.as_str(),
                id: None,
                dedup_key,
                suppressed: Some(suppressed),
                retry_after_secs: Some(retry_after),
            }),
        ));
    }

    // Record the timeline line first (so the 202 reflects a durable row), then
    // fan the alert out to the monitor's channels. The dispatch is spawned, so a
    // slow channel never holds up the producer.
    let alert_id = db::insert_pushed_alert(
        &state.pool,
        &id,
        severity.as_str(),
        &title,
        &message,
        dedup_key.as_deref(),
    )
    .await?;
    spawn_alert_dispatch(&state, monitor, severity, title.clone(), message);

    tracing::info!(monitor = %id, severity = severity.as_str(), %title, "pushed alert dispatched");
    Ok((
        StatusCode::ACCEPTED,
        Json(AlertResponse {
            status: "dispatched",
            severity: severity.as_str(),
            id: Some(alert_id),
            dedup_key,
            suppressed: None,
            retry_after_secs: None,
        }),
    ))
}

/// Fan a pushed alert out to a monitor's channels in the background, so a slow
/// channel never blocks the 202. The owned strings outlive the request.
fn spawn_alert_dispatch(
    state: &AppState,
    monitor: &Monitor,
    severity: AlertSeverity,
    title: String,
    message: String,
) {
    let notifier = state.notifier.clone();
    let name = monitor.name.clone();
    let notify = monitor.notify.clone();
    tokio::spawn(async move {
        notifier
            .load_full()
            .dispatch(
                Event::Alert {
                    monitor: &name,
                    severity,
                    title: &title,
                    message: &message,
                },
                notify.as_deref(),
            )
            .await;
    });
}

/// Build the stored/sent alert message: the producer's text, then each tag as a
/// `key=value` line (sorted, so the same alert always reads identically). Every
/// part is trimmed and bounded.
fn render_alert_message(message: &str, tags: &std::collections::HashMap<String, String>) -> String {
    let mut out: String = message.trim().chars().take(MAX_PUSH_MSG_CHARS).collect();
    let mut pairs: Vec<(&String, &String)> = tags.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in pairs.into_iter().take(MAX_ALERT_TAGS) {
        let key: String = key.trim().chars().take(MAX_ALERT_TAG_CHARS).collect();
        if key.is_empty() {
            continue;
        }
        let value: String = value.trim().chars().take(MAX_ALERT_TAG_CHARS).collect();
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&key);
        out.push('=');
        out.push_str(&value);
    }
    out
}

#[derive(Debug, Deserialize)]
pub(crate) struct SilenceQuery {
    /// Comma-separated monitor ids, or `all` (stored as the `*` wildcard).
    monitors: String,
    /// How long to mute, e.g. `10m`, `1h30m`. Capped at 7 days.
    duration: String,
    #[serde(default)]
    reason: Option<String>,
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
        ("duration" = String, Query, description = "How long to mute (e.g. 10m, 1h30m; max 7d)"),
        ("reason" = Option<String>, Query, description = "Optional note recorded with the silence"),
        ("token" = Option<String>, Query, deprecated, description = "Viewer token; deprecated on this endpoint (answered with a Deprecation header): use Authorization: Bearer")
    ),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Alerts muted until the returned time", body = SilenceResponse),
        (status = 400, description = "Unparseable duration or empty monitor list"),
        (status = 401, description = "Missing or wrong token, or no auth_token configured"),
        (status = 404, description = "Unknown monitor id")
    )
)]
pub(crate) async fn silence(
    State(state): State<AppState>,
    // Muting alerts is an operator action: it strictly requires the
    // configured viewer token.
    Operator { config }: Operator,
    Query(query): Query<SilenceQuery>,
) -> Result<Json<SilenceResponse>, AppError> {
    let silenced = silence::apply(
        &state.pool,
        &config,
        &query.monitors,
        &query.duration,
        query.reason.as_deref(),
    )
    .await
    .map_err(|err| match err {
        SilenceError::InvalidDuration => {
            AppError::BadRequest("invalid duration (use e.g. 10m, 1h30m; max 7d)")
        }
        SilenceError::NoIds => AppError::BadRequest("no monitor ids given"),
        SilenceError::UnknownId(_) => AppError::NotFound("unknown monitor id"),
        SilenceError::Database(err) => AppError::Internal(err.into()),
    })?;
    let (monitors, until) = (silenced.ids, silenced.until);
    tracing::info!(monitors = ?monitors, until, "alerts silenced via API");
    Ok(Json(SilenceResponse { monitors, until }))
}

#[derive(Deserialize)]
pub(crate) struct BadgeParams {
    style: Option<Style>,
}

impl BadgeParams {
    fn style(self) -> Style {
        self.style.unwrap_or(Style::Flat)
    }
}

#[utoipa::path(
    get,
    path = "/api/badge/{id}/status",
    params(
        ("id" = String, Path, description = "Monitor id"),
        ("style" = Option<String>, Query, description = "Badge style: flat, flat-square, or for-the-badge")
    ),
    responses(
        (status = 200, description = "Status badge (SVG)"),
        (status = 404, description = "Unknown monitor")
    )
)]
pub(crate) async fn status_badge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<BadgeParams>,
) -> Result<impl IntoResponse, AppError> {
    // Badges are embeddable and unauthenticated: a private monitor's badge is
    // a 404, not a leak. They answer from two indexed single-monitor reads
    // (milliseconds) rather than the page summary - a badge on a README must
    // not pay for, or wait on, a full status-page rebuild.
    let config = state.config.borrow().clone();
    badge_monitor(&config, &id)?;
    let threshold = i64::from(config.alerts.fail_threshold.max(1));
    let recent = db::recent_checks(&state.pool, &id, threshold).await?;
    let status = db::derive_status(&recent, threshold);
    Ok(svg_response(badge(
        "status",
        status,
        status_color(status),
        params.style(),
    )))
}

#[utoipa::path(
    get,
    path = "/api/badge/{id}/uptime",
    params(
        ("id" = String, Path, description = "Monitor id"),
        ("style" = Option<String>, Query, description = "Badge style: flat, flat-square, or for-the-badge")
    ),
    responses(
        (status = 200, description = "24h uptime badge (SVG)"),
        (status = 404, description = "Unknown monitor")
    )
)]
pub(crate) async fn uptime_badge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<BadgeParams>,
) -> Result<impl IntoResponse, AppError> {
    // Same single-monitor fast path as `status_badge`; the permille arithmetic
    // mirrors the summary's card so both always show the same figure.
    let config = state.config.borrow().clone();
    badge_monitor(&config, &id)?;
    let since = Utc::now().timestamp() - hora_core::SECONDS_PER_DAY;
    let (available, total) = db::availability(&state.pool, &id, since).await?;
    let permille = (total > 0).then(|| available.saturating_mul(1000) / total);
    let (message, color) = match permille {
        Some(permille) => (hora_core::fmt::permille(permille), uptime_color(permille)),
        None => ("n/a".to_owned(), "#9f9f9f"),
    };
    Ok(svg_response(badge(
        "uptime",
        &message,
        color,
        params.style(),
    )))
}

/// The badge-visible monitor with `id`: badges are always the public view,
/// whoever embeds them - a private monitor's badge is indistinguishable from
/// an unknown one.
fn badge_monitor<'a>(config: &'a Config, id: &str) -> Result<&'a Monitor, AppError> {
    visible_monitor(config, &Visibility::new(config, &Audience::Public), id)
}

/// Sample a series down to at most `max` points, keeping its overall shape.
pub(crate) fn downsample(points: Vec<Point>, max: usize) -> Vec<Point> {
    if points.len() <= max || max == 0 {
        return points;
    }
    let step = points.len().div_ceil(max);
    points.into_iter().step_by(step).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn badges_expose_public_monitors_only() {
        let config: Config = toml::from_str(
            r#"
            [page]
            [server]
            [[monitors]]
            id = "pub"
            name = "Public"
            target = "https://example.com"
            interval_secs = 60
            [[monitors]]
            id = "priv"
            name = "Private"
            target = "https://internal.example.com"
            interval_secs = 60
            public = false
        "#,
        )
        .unwrap();
        assert!(badge_monitor(&config, "pub").is_ok());
        // Private and unknown are the same 404 - the badge must not leak
        // which private ids exist.
        assert!(badge_monitor(&config, "priv").is_err());
        assert!(badge_monitor(&config, "nope").is_err());
    }

    #[test]
    fn alert_message_folds_tags_in_sorted_order() {
        let tags = HashMap::from([
            ("task_id".to_owned(), "t1".to_owned()),
            ("patch_id".to_owned(), "9f3c".to_owned()),
        ]);
        // Producer text first, then each tag as `key=value`, keys sorted so the
        // same alert always reads identically.
        assert_eq!(
            render_alert_message("3/12 ops failed", &tags),
            "3/12 ops failed\npatch_id=9f3c\ntask_id=t1"
        );
    }

    #[test]
    fn alert_message_handles_empty_text_and_tags() {
        assert_eq!(render_alert_message("  boom  ", &HashMap::new()), "boom");
        // No producer text: the message is just the tag lines.
        let tags = HashMap::from([("k".to_owned(), "v".to_owned())]);
        assert_eq!(render_alert_message("", &tags), "k=v");
        // Nothing at all yields an empty message.
        assert_eq!(render_alert_message("", &HashMap::new()), "");
    }

    #[tokio::test]
    async fn collect_visible_widens_until_enough_rows_are_visible() {
        // 100 rows, newest first; only every 10th is "visible".
        let rows: Vec<i64> = (0..100).collect();
        let mut fetched = Vec::new();
        let visible = collect_visible(
            5,
            |window| {
                fetched.push(window);
                let page: Vec<i64> = rows
                    .iter()
                    .copied()
                    .take(usize::try_from(window).unwrap())
                    .collect();
                async move { Ok(page) }
            },
            |page: &mut Vec<i64>| page.retain(|row| row % 10 == 0),
        )
        .await
        .ok()
        .expect("rows");
        // Filtering happened before the limit: five visible rows, not the
        // one or two a "take 5 then filter" would leave.
        assert_eq!(visible, vec![0, 10, 20, 30, 40]);
        assert_eq!(fetched, vec![5, 20, 80]);
    }

    #[tokio::test]
    async fn collect_visible_stops_when_the_table_is_exhausted() {
        let visible = collect_visible(
            50,
            |_window| async { Ok(vec![1_i64, 2, 3]) },
            |page: &mut Vec<i64>| page.retain(|row| *row != 2),
        )
        .await
        .ok()
        .expect("rows");
        assert_eq!(visible, vec![1, 3]);
    }
}
