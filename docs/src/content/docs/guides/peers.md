---
title: Mutual surveillance (peers)
description: Who watches the watcher - dead-man heartbeats between Hora nodes, quorum, and external receivers.
---

A monitor is only as good as the machine it runs on. Two Hora nodes (two
Raspberry Pi at two friends' places, a VPS and a homelab...) can watch each
other with **dead-man heartbeats**: each node pings the other while it is
healthy; when the pings stop, the survivor alerts.

Nothing here is a bespoke protocol - every exchange is plain HTTP, so a peer
can be another Hora, a healthchecks.io / UptimeRobot endpoint, or a cron job.

## The outbound heartbeat: `[health]`

```toml
[health]
id = "hora-a"        # this node's identity (how peers refer to it)
interval_secs = 60   # heartbeat cadence while healthy
grace_secs = 180     # startup grace before a never-seen peer is alerted
# quorum = true      # see below
# heartbeat_url = "${HC_PING_URL}"   # optional extra dead-man target
```

The node POSTs to each peer's `ping_url` every `interval_secs` - but **only
while it is locally healthy** (its scheduler is ticking and its database is
writable). A hung process stops pinging, and the receiver notices.

## Peers: two independent halves

Each `[[peers]]` entry can declare either or both directions:

```toml
[[peers]]
id = "hora-b"                                    # the peer's [health].id
name = "Hora B (Paris)"
# OUT - I heartbeat the peer while I'm healthy:
ping_url = "https://b.example/api/push/hora-a"
ping_token = "${PEER_B_TOKEN}"                   # sent as X-Push-Token
# IN - I watch the peer and alert if it goes silent:
expect_every_secs = 90
listen_token = "${PEER_B_IN}"                    # required with expect_every_secs
# listen_id = "hora-b"                           # the push id it pings: /api/push/{listen_id} (default: id)
# witness_url = "https://b.example/healthz"      # default: origin(ping_url)/healthz
# notify = ["ops-telegram"]                      # route this peer's alerts
```

Watched peers appear in their own section on the status page (their state
does not roll into the overall badge: it tracks your services, not the
surveillance mesh). `[health]` and `[[peers]]` reload live like everything
else.

A watched peer follows the same rules as a monitor:

- It is down after `alerts.fail_threshold` missed heartbeats in a row.
- A peer that has **never** sent a heartbeat alerts too ("no heartbeat
  received yet") once `expect_every_secs` has passed since this node started
  watching it. That start is stored, so restarts do not reset it. For
  `grace_secs` after this node starts, peer alerts are held, so two nodes
  rebooting together do not page each other.
- It can be muted: a `[[maintenance]]` window or a silence that names its
  `listen_id` (`hora silence hora-b 30m "rebooting"`).
- Its alert state survives a config edit or a restart: no repeated down,
  no lost recovery.

## Quorum: outage or partition?

With three or more nodes, set `quorum = true`: before alerting a peer down,
the node asks the *other* peers' `/healthz` whether they still see it. If any
does, it is a network partition between you two - reported as a degraded peer
link, not a false "node down". A no-op with fewer than three nodes.

## Multi-vantage confirmation

The peers don't just watch *each other* - they can confirm **your monitors'
outages** from their vantage point. With

```toml
[health]
id = "hora-a"
confirm_with_peers = true   # per-monitor override: confirm_with_peers = true/false
```

a monitor that confirms down locally triggers one concurrent round of probe
requests to the peers before the alert goes out, and the alert carries the
verdict:

- *"confirmed down from 3/3 vantage points"* - a real outage;
- *"seen UP by hora-b - down from 1/3 vantage points (network issue near
  this node?)"* - probably your fibre, not the service.

Two Raspberry Pi at two homes become a distributed Pingdom.

### Local-only downs

When **every peer that answered sees the target up** (and at least one
answered), the down is *local-only*: a problem on the road from this node,
not an outage. It does **not** go to the monitor's channels. It goes to the
channels listed in `alerts.notify_unconfirmed`, or, without that key, it is
only recorded - the incident, the timeline, and the status page's
*Not an outage* notice.

```toml
[alerts]
notify_unconfirmed = ["ntfy-low"]   # optional: a quiet channel for local-only downs
```

Only an explicit contradiction quiets a down. No peer configured, no peer
reachable, peers that do not watch this target, or **any** peer seeing it
down too: the alert goes out normally - geo-partial outages are real
outages.

While a local-only down lasts, the node asks its peers again every 5
minutes. As soon as they no longer all see the target up (they see it down
too, or stop answering), the real down alert goes out, once, to the usual
channels. A recovery goes to whoever received a down: the quiet channels
for a down that stayed local-only, both after a late confirmation. The
[webhook](../alerting/#channels) payload carries `local_only: true` on the
`down` and `recovered` events of a local-only down.

Confirmation only applies to network probes (http, tcp, icmp, dns). Exec
and push monitors have no remote point of view, so they are never sent to
peers, and a peer asked to probe one answers 400.

The same round is available on demand from the terminal with
[`hora probe <id> --confirm`](../../reference/cli/#hora-probe): it prints the
verdict both ways - *"down for everyone or just me?"* when the target is down
here, and *"up from 3/3 vantage points"* when it is up.

### How a probe request works

The requester `POST`s to the peer's `/api/peer/probe` (derived from the
origin of `ping_url`), authenticated with the same token pair as heartbeats:
it presents its `ping_token`, which the responder checks against its
`listen_token` for that peer - **required** here; the id alone never
authorizes a probe.

The responder only probes targets **present in its own configuration**
(matched on kind + target, probed with its own timeout, assertions and
proxy) - never an arbitrary target, so a leaked token cannot turn a peer
into an SSRF relay. Both nodes must therefore know the monitor, which pairs
naturally with sharing the config in git.

### Failure behaviour

Strictly fail-open, by construction: peers being slow, unreachable, behind a
wrong token or unaware of the target never block the alert, never delay it
past a hard 10-second deadline (probes run concurrently), and never suppress
it. The worst possible outcome is an alert *without* the vantage annotation -
exactly what Hora sent before the feature. The incident record is written
before the peers are even consulted.

## Seeing from elsewhere

Every peer with a `ping_url` is also asked, once a minute, how it sees the
targets both nodes monitor. The status page then shows each shared monitor
from the other places ("Hora B: 220ms"). The exchange is
authenticated like a probe request, read-only and fail-open.

Confirmation only works for monitors both nodes have. To check that the
mesh agrees, run:

```sh
hora peers diff
```

It lists, per peer, the monitors (kind and target) that only one side has,
and exits non-zero on any difference or an unreachable peer, so it can gate
a config deploy in CI.

## External receivers

- **OUT-only peer** - a plain dead-man to an external service:

  ```toml
  [[peers]]
  id = "healthchecks"
  name = "healthchecks.io"
  ping_url = "https://hc-ping.com/<uuid>"
  ```

- **Be watched by UptimeRobot** - point a keyword monitor at this node's
  `/healthz` and match the keyword `ok` (the top-level `status` field is
  `ok` only while the node is fully healthy). No peer entry needed.
