---
title: Upgrading
description: How to upgrade Hora, what to check before the next release, and where the version notes live.
---

Hora stores everything in one SQLite file; schema migrations are embedded in
the binary and applied automatically on startup. Upgrading is: replace the
binary (or image), restart.

## Docker

```sh
docker pull ghcr.io/uplg/hora:latest
docker stop hora && docker rm hora
docker run -d --name hora --restart unless-stopped \
  -p 8787:8787 \
  -v "$PWD/hora-config:/etc/hora" \
  -v hora-data:/data \
  ghcr.io/uplg/hora:latest
```

Your history lives on the `hora-data` volume and survives upgrades.

## Before you upgrade

A consistent snapshot of the database is one command, safe while the daemon
runs:

```sh
hora backup /backups/hora-pre-upgrade.db
```

Then validate your config with the **new** binary:

```sh
docker run --rm -v "$PWD/hora-config:/etc/hora" ghcr.io/uplg/hora:latest check
```

As a rule, deploy a new binary before a config that uses its new keys: an
older binary refuses keys it does not know.

## 0.10 to 0.11

The next release is a hardening release. One schema migration applies by
itself and there is no new config key, but a few behaviours change:

- **Some configs no longer load.** Intervals and timeouts above 30 days, an
  `expected_status` outside 100 to 599, an unbracketed IPv6 tcp target, or a
  `dns_resolver` given as a hostname. See
  [Bounds](../configuration/#bounds). A hot reload that fails keeps the
  previous config.
- **Push monitors and peers that never sent a heartbeat now alert** ("no
  heartbeat received yet"). Comment out monitors for jobs that do not run
  yet. See [push monitors](../guides/monitors/#push-heartbeat).
- **A push with `status=down` alerts** with the job's message, after
  `fail_threshold`, instead of one interval late as a missed heartbeat.
- **`?token=` on write endpoints is deprecated.** Move scripts to
  `Authorization: Bearer` (or `X-Push-Token` for push and alert). See
  [Authentication](../reference/api/#authentication).
- **Pages are rate-limited per client IP.** Behind a reverse proxy, set
  `server.client_ip_header`, or every visitor shares one bucket. See
  [Rate limiting](../reference/api/#rate-limiting--security-headers).
- **Notification wording changed** on several channels (`API is DOWN`
  instead of `DOWN: API`; `[DOWN] API is DOWN` email subjects). Update any
  filter that matched the old text.
- **Atom readers** show existing incidents once more as new: entry ids now
  point at `/incident/{id}`.
- **`/healthz` answers 503** on a degraded node, so Docker's `HEALTHCHECK`
  marks it unhealthy.
- **Removed monitors keep their history for 7 days** before it is deleted.
  See [Removed monitors](../configuration/#removed-monitors).

## Version notes

Every version's notes, with what to check before rolling out, are in
[`UPGRADES.md`](https://github.com/uplg/hora/blob/main/UPGRADES.md). The
full history is in the [changelog](/hora/changelog/).
