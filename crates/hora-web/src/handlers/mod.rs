//! HTTP endpoint handlers and their request/response types, one submodule per
//! area; the `OpenAPI` document and the static/ops endpoints live here.

pub(crate) mod annotations;
pub(crate) mod api;
pub(crate) mod badges;
pub(crate) mod pages;
pub(crate) mod peer;
pub(crate) mod push;

use std::sync::{Arc, LazyLock};

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};

use hora_core::config::{Config, Monitor};
use hora_core::db::Point;
use hora_core::mesh::wire::{HealthReport, PeerSeen};

use crate::error::AppError;
use crate::summary::{DayCell, IncidentView, MaintenanceView, MonitorView, Summary};
use crate::visibility::{Audience, Visibility};
use crate::{AppState, FAVICON_SVG, FONT_WOFF2, summary_for};

// `paths(...)` below names the handlers unqualified: a module-qualified path
// would become the operation's `OpenAPI` tag. utoipa resolves each
// through the `__path_*` type it generates next to the handler.
use annotations::{__path_announce, __path_announce_clear, __path_post_event, __path_silence};
use api::{__path_latency_json, __path_summary_json};
use badges::{__path_heatmap_svg, __path_status_badge, __path_uptime_badge};
use peer::{__path_peer_monitors, __path_peer_probe};
use push::{__path_post_alert, __path_push};

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
        annotations::SilenceResponse,
        annotations::AnnounceResponse,
        annotations::AnnounceClearResponse,
        push::AlertRequest,
        push::AlertResponse,
        annotations::EventResponse,
        hora_core::mesh::wire::ProbeRequest,
        hora_core::mesh::wire::ProbeResponse,
        hora_core::mesh::wire::PeerMonitors,
        hora_core::mesh::wire::PeerMonitor,
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
    let report = hora_core::mesh::peer::report(&state.pool, &config, &state.last_tick).await;
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

/// The configured monitor `id`, if this audience may see it. A private
/// monitor answers exactly like a missing one (404) - its existence is not
/// revealed either way.
pub(super) fn visible_monitor<'a>(
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
