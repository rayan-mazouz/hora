# <img src="brand/assets/mark.svg" alt="" width="36" align="top"> Hora

[![CI](https://github.com/uplg/hora/actions/workflows/ci.yml/badge.svg)](https://github.com/uplg/hora/actions/workflows/ci.yml)
[![image](https://img.shields.io/badge/image-~15%20MB-0E5E68)](https://github.com/uplg/hora/pkgs/container/hora)
[![checks](https://img.shields.io/badge/checks-HTTP%20·%20TCP%20·%20ICMP%20·%20DNS%20·%20push%20·%20exec-0E5E68)](https://uplg.github.io/hora/guides/monitors/)
[![alerts](https://img.shields.io/badge/alerts-10%20channels-0E5E68)](https://uplg.github.io/hora/guides/alerting/)
[![wakes you for](https://img.shields.io/badge/wakes%20you%20for-real%20outages%20only-B4633F)](https://uplg.github.io/hora/concepts/)
[![license](https://img.shields.io/github/license/uplg/hora)](LICENSE)

**Hora keeps the hours, so you can sleep.** A small self-hosted uptime
monitor: one Rust binary checks your services, keeps their history in
SQLite, serves a calm status page and a JSON API, and alerts you only when
something has really broken. A failed check is retried, an outage needs
several failures in a row, peers in other places can confirm it, and ten
services down behind one database send one alert: the cause.

![The Hora status page](docs/public/screenshot.webp)
<!-- TODO(ui): replace with a fresh screenshot of the new status page. -->

Named after the Horai, the Greek keepers of the hours. Its owl dozes while
all is well.

## Quick start

A minimal `hora-config/config.toml`:

```toml
[page]
title = "My services"

[server]   # the image sets the bind address and the database path

[[channels]]
name = "ops"
type = "telegram"
token = "${HORA_TELEGRAM_TOKEN}"
chat_id = "123456"

[[monitors]]
id = "website"
name = "Website"
target = "https://example.com"
interval_secs = 60
```

Then:

```sh
docker run -d --name hora --restart unless-stopped -p 8787:8787 \
  -e HORA_TELEGRAM_TOKEN=123:abc \
  -v "$PWD/hora-config:/etc/hora" -v hora-data:/data \
  ghcr.io/uplg/hora:latest
```

The status page is at <http://localhost:8787/>. Edit the file and Hora
reloads it live. Check it with `docker exec hora hora check`, and test your
channels with `docker exec hora hora test-alert`. Every option is in
[`config.example.toml`](config.example.toml).

## Why not Uptime Kuma or Gatus

| | Hora | Uptime Kuma | Gatus |
|---|---|---|---|
| Runs as | one static binary, ~15 MB image | Node.js app, ~182 MB `2-slim` image ([tags](https://hub.docker.com/r/louislam/uptime-kuma/tags)) | Go binary, ~26 MB image |
| Configured in | a TOML file, reloaded live | the web UI; file config declined ([#270](https://github.com/louislam/uptime-kuma/issues/270)) | a YAML file |
| A cascade | one alert for the cause (`depends_on`) | one alert per monitor; dependencies requested since 2021 ([#1089](https://github.com/louislam/uptime-kuma/issues/1089)) | one alert per endpoint |
| Other places | peers confirm an outage before the alert | no multi-node mode | remote instances merge dashboards, no confirmation |
| Targets | error budgets, burn-rate alerts, monthly SLA report | uptime only | uptime only |

Checked against each project on 3 October 2026. Kuma and Gatus have far
more check types and notification services; the full comparison, with
Healthchecks, is in [Why Hora](https://uplg.github.io/hora/why-hora/).

## Documentation

**[uplg.github.io/hora](https://uplg.github.io/hora/)**

- [Getting started](https://uplg.github.io/hora/getting-started/) and
  [Configuration](https://uplg.github.io/hora/configuration/)
- [Concepts](https://uplg.github.io/hora/concepts/): how a failed check
  becomes one alert, or none
- Guides: [monitors](https://uplg.github.io/hora/guides/monitors/),
  [alerting](https://uplg.github.io/hora/guides/alerting/),
  [the status page](https://uplg.github.io/hora/guides/status-page/),
  [SLOs](https://uplg.github.io/hora/guides/slo/),
  [incidents](https://uplg.github.io/hora/guides/incidents/),
  [per-group pages](https://uplg.github.io/hora/guides/multi-tenant/),
  [peers](https://uplg.github.io/hora/guides/peers/),
  [importing from Uptime Kuma](https://uplg.github.io/hora/guides/import/)
- Reference: [CLI](https://uplg.github.io/hora/reference/cli/) and
  [HTTP API](https://uplg.github.io/hora/reference/api/)
- [Changelog](https://uplg.github.io/hora/changelog/),
  [roadmap](https://uplg.github.io/hora/roadmap/),
  [development](https://uplg.github.io/hora/development/)

## Upgrade

```sh
docker pull ghcr.io/uplg/hora:latest
docker stop hora && docker rm hora
# then the same docker run as above
```

History lives on the `hora-data` volume. Read
[`UPGRADES.md`](UPGRADES.md) before each release.

## Develop

```sh
cp config.example.toml config.toml   # then edit
cargo run -p hora
just gate                            # fmt, clippy, deny, audit, tests: what CI runs
```

Needs a C toolchain and `cmake`. The docs are in [`docs/`](docs/), the brand
in [`brand/`](brand/). MIT licence; the status page embeds Cal Sans under
the [SIL OFL](crates/hora-web/assets/OFL.txt).
