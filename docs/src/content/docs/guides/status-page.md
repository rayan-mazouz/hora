---
title: The status page
description: What visitors see on Hora's status page, what you see once signed in, and the other ways to read it.
---

Hora serves its status page itself, at `/`, rendered on the server with no
JavaScript framework. It is the same page for your visitors and for you;
what changes is how much it shows.

## What visitors see

A visitor without a token sees the **public** monitors (`public = true`, the
default) and nothing else:

- **The overall state** at the top, with the page title (`[page] title`) and
  the time of the last update.
- **Announcements**: banners you pinned with `hora announce`,
  `POST /api/announce` or `[[incidents]]` in the config, newest first. See
  [Announcements](../alerting/#announcements).
- **Maintenance**: the windows in progress and the monitors they cover.
- **Monitors, by group** (`group = "..."`): for each one, its state, its 24-hour
  uptime, the latest latency, a daily uptime bar over `[page] history_days`
  (90 by default), a latency chart, p95 and p99, and, when set, the
  certificate's days left and the [error budget](../slo/) left.
- **Root cause**: *"caused by db"* on a monitor that is down because of an
  upstream, *"impacts: api, web"* on the upstream itself.
- **From elsewhere**: with peers, each monitor both nodes watch shows how it
  looks from the other nodes ("Hora B: 220ms").
- **Peers**: the other Hora nodes this one watches, if any, with when each
  was last seen.

Failure reasons are reduced to a safe category for visitors ("HTTP 500",
"content check failed"): the full reason can contain response bodies, DNS
answers or the keyword you assert on. A monitor opts into publishing the
full detail with `public_error_detail = true`.

The page refreshes itself every 30 seconds. It reads well on a phone.

## What you see

With the operator token (`server.auth_token`), the same page also shows:

- **Private monitors** (`public = false`). Without the token, a private
  monitor is absent everywhere: page, API, badges, history and feed. Its
  endpoints answer 404, exactly like an id that does not exist.
- **Full failure detail** on every monitor, and the captured response of
  the last failure in the history.
- **Event markers** (`hora event "deploy api v2.3"`) as dashed lines on the
  latency charts.
- **Channel health** in `/api/summary` and `hora top`.

For a browser, the simplest way to present the token is a link with
`?token=...` (read-only views accept it). Keep that link private; it ends up
in browser history. Scripts should send `Authorization: Bearer` instead
(see [Authentication](../../reference/api/#authentication)).

A [group token](../multi-tenant/#group-tokens) sits in between: it reveals
everything about one group on `/status/{group}`, and nothing about the
others.

## Other pages

| Page | What it shows |
| --- | --- |
| `/history` | Confirmed incidents, newest first, with notes and latency heatmaps. See [Incidents & history](../incidents/). |
| `/history.atom` | The same incidents as an Atom feed, to follow in a feed reader. |
| `/incident/{id}` | One incident as a post-mortem, with the markdown ready to copy. |
| `/timeline` | One chronology of the last 7 days: downs and recoveries, events, pushed alerts, announcements and silences. |
| `/status/{group}` | One group's page. See [Per-group pages](../multi-tenant/). |
| `/report/{YYYY-MM}` | The printable monthly SLA report. |

Visitors see sanitized versions of each: public monitors only, safe failure
categories, no event markers or silences.

## In a terminal

`curl` (or `wget`, or any client that asks for `text/plain`) gets an aligned
plain-text page instead of HTML, on `/` and on `/status/{group}`:

```
$ curl -s https://status.example.com/
My services
===========

Overall: up (All systems operational)
Updated: 2026-10-03 14:32:00 UTC

  ● Website     99.9%      84ms
  ● API        100.0%     112ms
```

For a live view that also acts (announce, silence), use
[`hora top`](../../reference/cli/#hora-top).

## Embedding and machines

- **Badges**: `/api/badge/{id}/status` and `/api/badge/{id}/uptime` are SVG
  badges for a README. See [Badges](../../reference/api/#badges).
- **JSON**: `/api/summary` has everything the page shows.
- **Prometheus**: `/metrics`.

## Behind a reverse proxy

Hora serves plain HTTP and assumes nothing about its domain: put it behind
Caddy, nginx or Cloudflare. Two settings matter there:

- **`server.client_ip_header`**: every page is rate-limited per client IP.
  Without this setting, every visitor shares the proxy's address and its
  bucket. Set it to the header your proxy writes (`x-real-ip`, or
  `cf-connecting-ip` behind Cloudflare). See
  [Rate limiting](../../reference/api/#rate-limiting--security-headers).
- **HSTS** belongs to the proxy that terminates TLS.

## Settings

```toml
[page]
title = "My services"   # shown at the top and in the browser tab
history_days = 90       # days in the uptime bar
```

Per monitor: `group`, `public`, `public_error_detail`, `depends_on` (for
the cause and impact lines), `slo_uptime` (for the budget), and
`degraded_over_ms`.

<!-- TODO(ui): once the new status page ships, update this page: the hero
     sentence and owl, "Needs attention", folded groups, the five state shapes,
     the vantage-point wording for visitors, and fresh screenshots. -->
