//! The read-only JSON API and the Prometheus metrics.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;

use hora_core::db::{self, Point};

use super::{state_summary, visible_monitor};
use crate::auth::Viewer;
use crate::error::AppError;
use crate::metrics;
use crate::summary::Summary;
use crate::visibility::Visibility;
use crate::{AppState, MAX_LATENCY_HOURS, MAX_LATENCY_POINTS, SECONDS_PER_HOUR};

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

/// Sample a series down to at most `max` points, keeping its overall shape.
pub(crate) fn downsample(points: Vec<Point>, max: usize) -> Vec<Point> {
    if points.len() <= max || max == 0 {
        return points;
    }
    let step = points.len().div_ceil(max);
    points.into_iter().step_by(step).collect()
}
