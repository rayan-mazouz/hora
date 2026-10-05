---
title: Configuration
description: The config file, environment interpolation, live reload, and validation.
---

Everything lives in one TOML file, read from `$HORA_CONFIG` (default
`./config.toml`). The annotated
[`config.example.toml`](https://github.com/uplg/hora/blob/main/config.example.toml)
documents every option; this page covers the mechanics.

## Sections

| Section | What it holds |
| --- | --- |
| `[page]` | Status page title, days rendered in the uptime bars |
| `[server]` | Bind address, database path, CORS, rate limits, viewer token |
| `[[channels]]` | Named notification channels (Telegram, Discord, email, ...) |
| `[alerts]` | Fail threshold, cert expiry warning, retention, alert grouping, channel watchdog, the quiet channels for local-only downs (`notify_unconfirmed`) |
| `[[maintenance]]` | Scheduled windows that mute alerts |
| `[[incidents]]` | Manual announcements shown as a status page banner |
| `[[monitors]]` | The monitors themselves |
| `[health]` / `[[peers]]` | Mutual surveillance ([dead-man's switch](../guides/peers/)) |

## Secrets stay in the environment

Any `${VAR}` in the file is replaced with the environment variable at load:

```toml
[[channels]]
name = "ops"
type = "telegram"
token = "${HORA_TELEGRAM_TOKEN}"
chat_id = "123456"
```

An unset variable **fails the load**, naming the key and the variable, so a
missing `-e` is caught by `hora check` instead of by the first alert. Write
`${VAR:-}` for a value that may be empty, or `${VAR:-fallback}` for a
default. An empty channel secret (`token`, `webhook_url`, `url`, `pass`, ...)
fails the load too, and so does an empty *access token* (`auth_token`,
`admin_token`, `push_token`, `listen_token`, `ping_token`) instead of
silently meaning "no token required".

## Live reload - no blind window

To add, remove or change a monitor **without downtime**, just edit the config:

- **Bare metal / mounted directory:** Hora watches the file and reloads
  automatically.
- **Anywhere:** `kill -HUP <pid>` - in Docker, `docker kill -s HUP hora`.

On reload, unchanged monitors keep running untouched; only new, removed or
changed ones are started or stopped, and the notification channels are
rebuilt - adding a Telegram token takes effect live too. Existing checks never
pause, so there is no window where nothing is watching.

A few `[server]` settings are read once at startup and need a restart:
`bind`, `allowed_origins`, the rate limit (`rate_limit_burst`,
`rate_limit_refill_secs`), `client_ip_header` and `client_ip_trusted_hops`.

A reload that fails validation is refused: Hora logs why and keeps running
with the previous config.

## Validation

```sh
hora check
```

validates the configuration and exits non-zero on error - made for CI and
pre-deploy hooks. Validation is strict: inverted maintenance windows, cyclic
`depends_on` graphs, private monitors without a viewer token, empty tokens
and malformed cron schedules are all rejected at load, not discovered at
3 a.m. What is allowed but probably a mistake (a token under 16 characters,
a `timeout_secs` longer than `interval_secs`) is a warning: `hora check`
prints each one and counts them, and `hora check --strict` exits non-zero
on any.

### Bounds

Values that cannot work are refused too:

| Setting | Accepted |
| --- | --- |
| `interval_secs`, `timeout_secs`, `expect_every_secs`, `health.interval_secs` | at most 30 days (2592000) |
| `expected_status` | 100 to 599 |
| An IPv6 `tcp` target | bracketed: `[::1]:80`, not `::1:80` |
| `dns_resolver` | an IP and a port (`8.8.8.8:53`, `[2620:fe::fe]:53`), not a hostname |

A `timeout_secs` longer than `interval_secs` is accepted with a warning.

Before upgrading, run `hora check` with the **new** binary against your
config: a key or a value an older release accepted may be refused now (see
[Upgrading](../upgrading/)).

## Retention and downsampling

Raw checks are kept per monitor (`retention_days`, default
`alerts.default_retention_days = 90`). Alongside them, every ended hour is
rolled up into an hourly bucket; hourly buckets become daily ones after 90
days, kept for a year. The daily
uptime bars keep working beyond the raw retention window, and the database
never grows forever. Closed incidents age out after a year; expired silences
are swept too. The prune runs every 6 hours (the first one 5 minutes after
start) and deletes five minutes of checks per statement, pausing in
between, so the checks being recorded never wait on it. A tick spends at
most two minutes deleting: after lowering a retention a lot, the backlog is
worked off over the following ticks, five minutes apart, with the hourly
roll-ups in between. The WAL file
(`<db>-wal`) is truncated back to 64 MiB after each checkpoint.

### Removed monitors

Taking a monitor out of the config does not delete its history at once.
Hora notes when the id disappeared, logs a warning that names it and the
date its data will go, and deletes its checks, aggregates and incidents
**7 days** later. Put the id back within the week (after a rename, or a
monitor commented out for an afternoon) and everything is kept. Watched
peers are treated the same way, by their `listen_id`.

For backups, `hora backup <dest>` snapshots the live database with SQLite's
`VACUUM INTO` - see the [CLI reference](../reference/cli/#hora-backup).
