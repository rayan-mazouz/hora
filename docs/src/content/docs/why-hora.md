---
title: Why Hora
description: How Hora compares with Uptime Kuma, Gatus and Healthchecks, and when one of them is the better choice.
---

Hora is built around one promise: **it wakes you only for a real outage**.
Everything else follows from that: checks are retried, an outage needs
several failures in a row, peers in other places can confirm it, and a
cascade sends one alert about its cause. It ships as one small binary with a
TOML file, so the whole setup fits in git.

Three other self-hosted tools cover similar ground, and each is a good
choice for some teams. This page says plainly where they differ.

:::note[How this was checked]
The columns for the other tools were checked against their own READMEs,
documentation, source trees and issue trackers on **3 October 2026**, at
Uptime Kuma 2.5.5, Gatus v5.37.0 and Healthchecks v4.4. Projects move: if a
line is out of date, please [open an issue](https://github.com/uplg/hora/issues).
Image sizes are the compressed amd64 sizes Docker Hub reports.
:::

## At a glance

<div class="hora-compare">

| | Hora | Uptime Kuma | Gatus | Healthchecks |
| --- | --- | --- | --- | --- |
| Written in | Rust, one static binary | Node.js | Go, one binary | Python (Django) |
| Docker image | ~15 MB | ~602 MB (`2`), ~182 MB (`2-slim`) | ~26 MB | ~103 MB |
| Configured in | A TOML file, reloaded live | The web UI, stored in its database | A YAML file | The web UI or its API |
| Storage | SQLite | SQLite, MariaDB or MySQL | Memory, SQLite or Postgres | SQLite, Postgres, MySQL or MariaDB |
| Active checks | HTTP, TCP, ICMP, DNS, exec plugins | Many: HTTP, TCP, ping, DNS, databases, gRPC, MQTT, SNMP and more | HTTP, TCP, UDP, ICMP, DNS, SSH, TLS, gRPC and more | None: it receives pings |
| Heartbeats from your jobs | Yes, interval or cron schedule | Yes, interval | Yes, interval ("external endpoints") | Yes, interval, cron or OnCalendar |
| N failures before an alert | Yes (`fail_threshold`) | Yes (retries) | Yes (`failure-threshold`) | No, a grace time instead |
| One alert for a cascade | Yes, by `depends_on` | No | No | No |
| Confirmed from other places | Yes, by peers | No | No | Not applicable |
| Error budget, burn-rate alerts | Yes | No | No | No |
| Monthly SLA report | Yes, printable | No | No | No |
| Status page | Yes, plus one per group | Yes, several | One dashboard per instance | No (badges only) |
| Notification channels | 10 | 90+ | About 40 | About 28 |
| Licence | MIT | MIT | Apache-2.0 | BSD-3-Clause |

</div>

## Uptime Kuma

[Uptime Kuma](https://github.com/louislam/uptime-kuma) is the most popular
of the four, and for good reasons: a friendly web UI, a very long list of
check types (databases, MQTT, game servers, a real browser) and more than
ninety notification services.

Where Hora differs:

- **Configuration as code.** Kuma is configured in its UI and keeps the
  result in its database. A request for file-based configuration
  ([#270](https://github.com/louislam/uptime-kuma/issues/270)) was closed
  without being implemented. Hora's only source of truth is a TOML file you
  can review, diff and share between nodes.
- **Cascades.** Kuma can group monitors under a parent, but a down database
  still sends one alert per dependent service. Dependent monitoring is an
  open request ([#1089](https://github.com/louislam/uptime-kuma/issues/1089),
  since 2021). Hora folds them into one alert about the cause.
- **Several places.** Kuma has no multi-node mode; multi-region checks
  ([#5024](https://github.com/louislam/uptime-kuma/issues/5024)) were closed
  as not planned. Its Globalping monitor runs a check from third-party
  probes. Hora nodes confirm each other's outages before alerting.
- **Footprint.** Kuma 2's image is about 602 MB (182 MB for `2-slim`);
  Hora's is about 15 MB.

**Choose Uptime Kuma** if you want to set everything up by clicking, or you
need one of its many check types or notification services.

Coming from Kuma? [`hora import kuma`](../guides/import/) converts a backup.

## Gatus

[Gatus](https://github.com/TwiN/gatus) is the closest in spirit: a single Go
binary, a YAML file, a failure threshold before alerting, and many check
types including UDP, SSH and gRPC.

Where Hora differs:

- **Cascades.** Gatus alerts per endpoint; it has no notion of one endpoint
  depending on another.
- **Several places.** Gatus's "remote instances" feature (marked
  experimental) pulls other instances' results into one dashboard. It does
  not confirm a failure from another place before alerting. Hora asks its
  peers to probe the same target first, and says in the alert whether the
  outage was seen from everywhere or only from here.
- **Targets.** Gatus shows uptime; Hora also tracks an error budget per
  monitor, sends burn-rate alerts, and writes a monthly SLA report.
- **Status pages.** One Gatus instance serves one dashboard; the maintainer
  suggests running several instances for several pages
  ([#1035](https://github.com/TwiN/gatus/issues/1035)). Hora serves one page
  per group, with a token per client.
- **Storage.** Gatus keeps results in memory by default (SQLite or Postgres
  optional). Hora always writes to SQLite and rolls old data up into hourly
  and daily buckets.

**Choose Gatus** if you need UDP, SSH or gRPC checks, multi-step check
suites, or a notification service Hora lacks.

## Healthchecks

[Healthchecks](https://github.com/healthchecks/healthchecks) does one thing
very well: it watches **your jobs**. Cron jobs and timers ping it, and it
alerts when a ping is late or reports a failure. It understands cron
expressions with time zones and systemd `OnCalendar` schedules.

It never probes anything itself, so it is not an uptime monitor in the same
sense: it has no HTTP checks, no status page and no latency.

Hora covers the common part: a push monitor with a five-field cron
`schedule` (UTC) and a `grace_secs`, next to active checks, on the same
page. Healthchecks goes further on schedules (time zones, `OnCalendar`) and
on the job's lifecycle (start signals, run times).

**Choose Healthchecks** if what you watch is mostly scheduled jobs.
The two also combine well: a Hora node can send its own
[dead-man heartbeat](../guides/peers/#external-receivers) to a Healthchecks
check, so something outside watches the watcher.

## What Hora does not do

To be fair in the other direction:

- **Fewer check types.** No UDP, SSH, gRPC (declined, see the
  [roadmap](../roadmap/#deliberately-not-planned)), database or browser
  checks built in. An [exec probe](../guides/monitors/#exec) runs any
  monitoring-plugins check, which covers many of them.
- **Ten notification channels**, not ninety. The generic webhook reaches the
  rest.
- **No web editor.** Everything is in the TOML file, by design.
- **No multi-step scenarios** (log in, then check). Declined, to keep the
  configuration simple.
- **SQLite only.** One file, easy to back up, but no shared database server.
