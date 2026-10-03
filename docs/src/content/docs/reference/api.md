---
title: HTTP API
description: Endpoints, authentication, rate limiting, security headers and badges.
---

Everything the status page knows is also available as JSON, plus push
heartbeats and ad-hoc silencing. A generated **OpenAPI 3.1** document lives at
`/api/openapi.json` - point any client (Bruno, Insomnia, Scalar, Swagger
Editor...) at it.

## Endpoints

| Endpoint | Description |
| --- | --- |
| `GET /` | The HTML status page - or an aligned plain-text rendering for curl/wget. |
| `GET /metrics` | Prometheus metrics (text exposition format). |
| `GET /history` | Incident history page (HTML). |
| `GET /history.atom` | Incident history as an Atom feed. |
| `GET /status/{group}` | Status page restricted to one display group ([group tokens](../../guides/multi-tenant/) accepted). |
| `GET /incident/{id}` | One incident as a post-mortem page, with the raw markdown to copy. Visitors get the sanitized view; a private monitor's incident is a 404. |
| `GET /timeline` | The unified chronology of the last 7 days: downs and recoveries, events, pushed alerts, announcements, silences. Visitors see public incidents and announcements only. |
| `GET /report/{YYYY-MM}` | Printable monthly SLA report; `?group=` scopes it to one group. Months older than the 12-month aggregate retention answer 400. |
| `GET /api/summary` | All monitors: status, 24h uptime (per-mille), p50/p95/p99 latency, cert days left, daily history; plus active incidents. |
| `GET /api/monitors/{id}/latency?hours=24` | Latency samples `[{ "t", "latency_ms" }]` (404 if unknown). |
| `POST /api/push/{id}` | Record a heartbeat for a push monitor or a watched peer. |
| `POST /api/monitors/{id}/alert` | Push an ad-hoc alert to a monitor's channels (a producer's own failure); records a timeline line, never changes the monitor's status. |
| `POST /api/silence` | Mute alerts ad hoc (deploy hook). Requires `server.auth_token`. |
| `POST /api/event` | Record an event marker ("deploy api v2.3"), correlated into incidents. Requires `server.auth_token`. |
| `GET /api/monitors/{id}/heatmap.svg` | 28-day hours-by-days latency heatmap (SVG), colour relative to the monitor's median. |
| `POST /api/announce` | Pin a public status-page banner (`DELETE` clears); auto-expiry via `until`, a duration (`4h`) or a UTC time of day (`18:00`). Requires `server.auth_token`. |
| `POST /api/peer/probe` | [Multi-vantage confirmation](../../guides/peers/#multi-vantage-confirmation) between nodes: probe a target *from this node's own config* and answer with the verdict. Requires the requesting peer's `listen_token`. |
| `GET /api/peer/monitors?from=<peer-id>` | The mesh exchange behind `hora peers diff` and the per-vantage view: this node's probeable monitors with its own view of each. Same peer authentication as `/api/peer/probe`. |
| `GET /api/badge/{id}/status` | Embeddable SVG status badge. |
| `GET /api/badge/{id}/uptime` | Embeddable SVG 24h-uptime badge. |
| `GET /api/openapi.json` | The OpenAPI 3.1 spec, generated from the code. |
| `GET /healthz` | Health of this node and its view of watched peers, as JSON. Answers **503** when the node is degraded (stalled scheduler, unwritable database), 200 otherwise. |

## Authentication

Hora knows three kinds of token, each sent in a header:

| Token | Header | Opens |
| --- | --- | --- |
| `server.auth_token` (operator) | `Authorization: Bearer <token>` | Private monitors on every read view; announce, silence, event and alert writes. |
| A monitor's `push_token` | `X-Push-Token: <token>` | `POST /api/push/{id}` and `POST /api/monitors/{id}/alert` for that monitor. |
| A group token | `Authorization: Bearer <token>` | `/status/{group}` and `/report/{month}?group=` with that group's full detail. |

Without a token, read views serve the public subset only, and a private
monitor answers exactly like a missing one (404), so its existence is not
revealed.

**Query-string tokens.** Read-only views (the page, `/history`,
`/history.atom`, `/timeline`, `/metrics`, `/api/summary`, latency) also
accept `?token=`, which is handy for a kiosk screen or a feed reader that
cannot set headers. On **write** endpoints `?token=` is **deprecated**: it
still works, but the response carries `Deprecation: true` and a `Link` to
this section, and Hora logs one warning per endpoint. A token in a URL ends
up in proxy access logs, browser history and shell history - send the
header instead.

## `POST /api/push/{id}`

Record a heartbeat for a push monitor (or a watched peer). Send the token as
an `X-Push-Token` header (`?token=` still works but is
[deprecated](#authentication)):

```sh
curl -fsS -X POST -H "X-Push-Token: ${TOKEN}" \
  "https://status.example.com/api/push/nightly-backup?status=up&ping=42"
```

Optional query: `status=up|down|degraded` (default up), `msg=...` (recorded
with the heartbeat, bounded), `ping=<ms>`. Answers 401 on a wrong token, 404
if the id is not a push target, and 400 on an unknown `status` or a negative
`ping`.

The `status` is the job's verdict and drives alerting: `down` counts as a
failure with `msg` as the reason (confirmed down after `fail_threshold`),
`degraded` counts as degraded. Two pushes in the same second: the last one
wins. See [push monitors](../../guides/monitors/#push-heartbeat).

## `POST /api/monitors/{id}/alert`

Let a producer push its **own** failure straight to a monitor's notification
channels - "patch materialization failed", "nightly export wrote 0 rows" -
without it pretending to be a probe result. The alert fans out to the
monitor's `notify` channels immediately, adds a line to that monitor's
timeline (shown on `/history`), and **never** marks the monitor down: status
stays driven by probes/heartbeats alone.

Authenticate with the monitor's own `push_token` as an `X-Push-Token` header,
or with `server.auth_token` as `Authorization: Bearer` (`?token=` is
[deprecated](#authentication)).
The endpoint is closed unless one of those is configured and matches.

```sh
curl -fsS -X POST \
  -H "X-Push-Token: ${TOKEN}" -H "Content-Type: application/json" \
  "https://status.example.com/api/monitors/ekb-api/alert" -d '{
    "severity": "error",
    "title":    "Patch materialization failed",
    "message":  "patch 9f3c… — 3/12 operations failed: SourceFile not found",
    "dedup_key":"ekb-api:materialization",
    "tags":     {"task_id":"4711","patch_id":"9f3c"}
  }'
```

JSON body: `severity` (`info` default, `warning`, `error`, `critical`),
`title` (required), `message` (optional), `dedup_key` (optional) and `tags`
(optional map, folded into the message). Answers **202 Accepted** with
`{"status":"dispatched","id":…}`; a 400 on an empty title or unknown severity,
401/404 like the push endpoint.

**Severity → priority.** On backends that have a native priority - ntfy,
Pushover, Gotify - the severity maps onto it (so `critical` pages louder than
`info`); elsewhere it shows as a text label. The generic JSON `webhook`
channel receives it structured: `{"event":"alert","severity","title","message"}`.

**`dedup_key` + anti-flood.** When two alerts share a `dedup_key`, any repeat
inside `alerts.push_alert_window_secs` (default 300) is **coalesced** - dropped
and counted, not dispatched again - and answers `202` with
`{"status":"coalesced","suppressed":N,"retry_after_secs":…}`. The rate-limiting
thus lives in Hora, so a flapping producer pages once and every producer
benefits without implementing its own throttle. Set the window to `0` to
dispatch every alert.

## `POST /api/silence`

Mute alerts for some monitors ad hoc - made for CI deploy hooks:

```sh
curl -fsS -X POST -H "Authorization: Bearer $HORA_TOKEN" \
  "https://status.example.com/api/silence?monitors=api,web&duration=10m&reason=deploy"
```

`monitors` is a comma-separated list of monitor ids or watched peers'
`listen_id`s, or `all` (silences mute peer-watch alerts too); `duration` looks like
`10m` / `1h30m` (max 7 days); `reason` is optional. **Strictly requires
`server.auth_token`** - muting alerts is an operator action, so without a
configured token the endpoint is closed. Unknown ids answer 404 (a typo'd
hook fails loudly), an unparseable duration 400. Checks keep recording; only
alerting is muted.

## `POST /api/announce`

Pin a public banner on the status page; `DELETE /api/announce` clears them
all. Requires `server.auth_token`.

```sh
curl -fsS -X POST -H "Authorization: Bearer $HORA_TOKEN" \
  "https://status.example.com/api/announce?title=Fibre+incident&body=ETA+6pm&severity=warning&until=18:00"
```

`severity` is `info` (default), `warning`, `critical` or `resolved`. `until` is a duration
(`4h`, `90m`) or a time of day in UTC (`18:00`, the next occurrence); without
it the banner stays until cleared. See
[Announcements](../../guides/alerting/#announcements).

## `GET /metrics`

Prometheus text format. Per monitor: `hora_monitor_status{id,name,status}`
(set for every monitor, including one whose status is still unknown),
`hora_monitor_up` and `hora_monitor_degraded` (omitted while the status is
unknown, so a restart never reads as an outage),
`hora_monitor_uptime_ratio`, `hora_monitor_last_latency_ms`,
`hora_monitor_latency_ms{quantile}` (a gauge) and `hora_cert_expiry_days`.
Private monitors need the operator token.

## Rate limiting & security headers

Every route except static assets and `/api/openapi.json` is **rate-limited
per client IP**, with `x-ratelimit-*` / `retry-after` headers. The `/api/*`
endpoints use `rate_limit_burst` / `rate_limit_refill_secs`; pages, badges,
reports, heatmaps, `/metrics` and `/healthz` get four times the burst and
refill. These settings are read once at startup.

**Client IP behind a proxy.** By default the client is the TCP peer, never a
forwarded header a direct client could forge. Behind a reverse proxy that
means every visitor shares the proxy's address - and its bucket - so tell
Hora which header your proxy sets:

```toml
[server]
client_ip_header = "x-real-ip"          # nginx: proxy_set_header X-Real-IP $remote_addr;
# client_ip_header = "cf-connecting-ip" # behind Cloudflare
```

Only name a header your proxy overwrites, and block direct access to the
origin. Hora takes the first address of the header, so `x-forwarded-for`
is safe only with a proxy that replaces it (Caddy's default) rather than
appending to what the client sent (nginx's `$proxy_add_x_forwarded_for`).

`allowed_origins` controls CORS (empty = allow any, since the data is
read-only and public). Responses carry a strict CSP,
`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
`Permissions-Policy` and `Cross-Origin-Opener-Policy`, plus an
`x-request-id` (a well-formed inbound one is honoured, otherwise minted)
echoed on the response for log correlation.

**HTTPS and HSTS** belong to the reverse proxy that terminates TLS: Hora
serves plain HTTP. Set `Strict-Transport-Security` there, for example
`header Strict-Transport-Security "max-age=31536000"` in Caddy or
`add_header Strict-Transport-Security "max-age=31536000" always;` in nginx.

## Badges

Embed a monitor's live status and 24h uptime in a README, by its config `id`:

```md
![status](https://status.example.com/api/badge/web/status)
![uptime](https://status.example.com/api/badge/web/uptime)
```

Badges use `flat` by default and accept `?style=flat-square` or
`?style=for-the-badge`. They are green when up / uptime is high, amber for
minor incidents, and red for an outage. Badges are embeddable and
unauthenticated; a private monitor's badge is a 404, not a leak.
