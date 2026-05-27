# node

Binary that runs on the Raspberry Pi. Captures 802.11p / ITS-G5 frames,
serves them to clients over BLE and HTTP, and on request replays or
injects archived traffic back onto the air.

The only crate with hardware dependencies (`libpcap`, `bluer`).

## Two modes

The mode is set via `mode = "Local"` or `mode = "Cloud"` in
`config.toml`, or via the `CITES_MODE` env var.

- **Local** opens the capture interface, runs the BLE GATT server (Linux
  only) and the HTTP API.
- **Cloud** has no capture hardware. It reads previously stored archives
  from `./captures/` and serves a tracking report on
  `GET /tracking/report`. Used by the docker-compose deployment.

## Modules

- `capture.rs` `CaptureDispatcher` opens the monitor-mode interface and
  fans each frame to three channels:
  - archive (unbounded, lossless, written by `PcapStorageManager`),
  - live ring buffer (250 messages, oldest dropped, for BLE clients),
  - replay channel (64 messages, oldest dropped, for the replay engine).
- `replay.rs` `ReplayEngine` re-injects matching live frames via
  `pcap::sendpacket`. Filters and TX power are configurable at runtime
  via `POST /replay/config` or BLE write.
- `injection.rs` `InjectionEngine` replays a stored PCAPNG archive on a
  configurable schedule (once, N times, infinite) with optional jitter
  and a dry-run mode.
- `config.rs` `AppConfig` (static, from `config.toml` plus `CITES_` env
  vars) and `RuntimeConfig` (log level and API port, pushed by the UI
  and persisted on disk so it survives restarts).
- `logger.rs` tracing subscriber with a rotating file appender at
  `/var/cites-node/cites-node.log` on Linux.
- `main.rs` startup, BLE GATT server, Axum HTTP router.

## HTTP API

Default port `8080`, CORS open.

| Endpoint | Method | Local | Cloud |
|---|---|---|---|
| `/status` | GET | x | x |
| `/metrics` | GET (CSV) | x | |
| `/replay/count` | GET | x | |
| `/replay/config` | POST | x | |
| `/pcap/stream` | GET (SSE) | x | |
| `/injection/start` | POST | x | |
| `/injection/stop` | POST | x | |
| `/injection/pause` | POST | x | |
| `/injection/filter` | POST | x | |
| `/injection/status` | GET | x | |
| `/node/config` | GET / POST | x | x |
| `/archive/list` | GET | x | x |
| `/archive/latest` | GET | x | x |
| `/archive/download/{f}` | GET | x | x |
| `/archive/upload/{f}` | POST | x | x |
| `/tracking/report` | GET | | x |

## BLE GATT (Linux only)

Service UUID `c17e5000-babe-4b1e-8256-000000000000`. Characteristics for
metrics, PCAP notifications, command writes, archive list, injection
status, node config, and an application-layer handshake that negotiates
the per-session chunk size (max 509 bytes, fallback 244).

## Capture interface

The default `config.toml` uses `mon0` as the monitor interface and the
same as the injection interface. For real OCB transmission set
`injection_interface = "wlan0"` (or whatever your ITS-G5 OCB device is)
so frames go out on RF while `mon0` keeps the full Radiotap metadata
for analysis.

## Running

```
RUST_LOG=info cargo run --package node
```

On a deployed Pi the binary runs as a systemd service (see
`deploy_node.sh` and the root README for details). Logs end up in
`/var/cites-node/cites-node.log`.
