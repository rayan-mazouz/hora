//! The peer-mesh endpoints: confirmation probes and the monitor listing.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use chrono::Utc;
use serde::Deserialize;

use hora_core::config::{Kind, Monitor};
use hora_core::db::{self};

use crate::AppState;
use crate::auth::authorize_peer;
use crate::error::AppError;

#[utoipa::path(
    post,
    path = "/api/peer/probe",
    request_body = hora_core::mesh::wire::ProbeRequest,
    security(("push_token" = [])),
    responses(
        (status = 200, description = "This vantage's verdict on the target", body = hora_core::mesh::wire::ProbeResponse),
        (status = 400, description = "Exec and push monitors are not network probes"),
        (status = 401, description = "Unknown requesting peer, or missing/wrong X-Push-Token"),
        (status = 404, description = "The target is not in this node's configuration")
    )
)]
pub(crate) async fn peer_probe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<hora_core::mesh::wire::ProbeRequest>,
) -> Result<Json<hora_core::mesh::wire::ProbeResponse>, AppError> {
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
        hora_core::mesh::confirm::PROBE_DEADLINE,
        hora_core::probe::run(&client, &probe_monitor),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_elapsed) => {
            return Ok(Json(hora_core::mesh::wire::ProbeResponse {
                up: false,
                error: Some("probe timed out at this vantage".to_owned()),
            }));
        }
    };
    Ok(Json(hora_core::mesh::wire::ProbeResponse {
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
        (status = 200, description = "This node's probeable monitors, with its own view of each (status, 24h median)", body = hora_core::mesh::wire::PeerMonitors),
        (status = 401, description = "Unknown requesting peer, or missing/wrong X-Push-Token")
    )
)]
pub(crate) async fn peer_monitors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PeerMonitorsQuery>,
) -> Result<Json<hora_core::mesh::wire::PeerMonitors>, AppError> {
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
        .map(|monitor| hora_core::mesh::wire::PeerMonitor {
            kind: monitor.kind,
            target: monitor.target.clone(),
            status: recent.get(&monitor.id).map_or_else(
                || "unknown".to_owned(),
                |checks| db::derive_status(checks, threshold).to_owned(),
            ),
            p50_ms: percentiles.get(&monitor.id).map(|(p50, _, _)| *p50),
        })
        .collect();
    Ok(Json(hora_core::mesh::wire::PeerMonitors { monitors }))
}
