# CITES

C-ITS Exploration System. A cross-platform Rust project for capturing,
decoding and analyzing ETSI C-ITS / ITS-G5 traffic in real time.

An edge node (Raspberry Pi) sniffs 802.11p frames off the air. Desktop,
mobile and web clients connect to it over BLE or IP, view the live
stream on a map, can track cars, warn against tracking via text-to-speech and serves as a blackbox.

Built as a Bachelor thesis at TU Darmstadt.

## Layout

Cargo workspace. All crates live under `packages/`:

```
core       parsing, ASN.1, BLE protocol, ego-MAC, vehicle tracking
node       Raspberry Pi binary (libpcap, bluer)
api        shared types and the single Dioxus server function
platform   client-side services (BLE / IP connection, TTS, signals)
ui         shared Dioxus views and components
desktop    desktop entry point
mobile     iOS / Android entry point
web        browser entry point
```

Workspace root has the build tooling:

- `deploy_node.sh` builds an ARM64 binary in Docker, SCPs it to the Pi,
  installs the systemd service. Pass `--force-asn` to regenerate the
  ASN.1 C sources first.
- `Dockerfile.node` native ARM64 build on Apple Silicon.
- `Dockerfile.node.cross` cross build from an x86 host.
- `docker-compose.yml` runs the node in Cloud mode (work in progress,
  not production ready).

## Stack

Rust 2021/2024, Dioxus 0.7 for the UI, Axum for HTTP, Tokio runtime,
btleplug on clients, bluer on the node (Linux only), pcap 2.4 for
capture and injection, sysinfo for the metrics, MapLibre GL JS for the
map, `tts` for native speech.

ASN.1 schemas are compiled from ETSI sources to C via asn1c, then
glued into Rust with `cc` + `bindgen` at build time.

## Requirements
- OCB mode kernel patch and hardware setup: [Car2X-Capture](https://github.com/WernersGit/car2x-capture)
- Access to a public or private tileserver (setup script within the repo)

## Node modes

**Local.** Live capture, BLE GATT server, full HTTP API. Default for the
deployed Pi.

**Cloud.** No capture hardware. Reads `./captures/*.pcapng`, exposes a
tracking report on `GET /tracking/report` plus the archive endpoints.
Used by the docker-compose deployment. Not production ready yet.

## Pipeline highlights

**Packet fanout.** `CaptureDispatcher` sends each frame to three
independent channels (archive, live BLE, replay). Lossless archive,
lossy ring buffers for the other two.

**ASN.1 decoding.** Generated C sources under
`packages/core/resources/asn1/<module>/` cover CAM (v1 + v2), DENM,
CPM, MAPEM, SPATEM, IVIM, SREM, SSEM, VAM and helpers (security,
GeoNetworking, PKI, SAEM, charging protocols, MRS, RMO etc.).
Regenerate them with:

```
chmod +x packages/core/resources/asn1/build_asn1_c.sh
packages/core/resources/asn1/build_asn1_c.sh
```

**Ego-MAC.** Rolling RSSI window per MAC with a stability score
(median, std-dev, IQR, MAD). The most stable MAC is treated as the
host vehicle.

**Vehicle tracking.** `VehicleTracker` chains pseudonym MACs into a
single virtual ID using kinematic continuity (position, speed,
heading, standstill, accel).

**Tracking warnings.** `TrackingWarningChecker` fires once a foreign
vehicle has been visible for too long (`min_visible_minutes`) or has
been followed for too far (`min_visible_km`). Both windows reset after
a time or position gap larger than the configured tolerance. Warnings
are spoken via the TTS queue.

**Replay.** `ReplayEngine` re-injects matching live frames. Filters
(vehicle ID, protocol bitmask, delay, TX power) can be changed at
runtime.

**Injection.** `InjectionEngine` reads a stored archive and re-transmits
it according to a schedule (`Once`, `Count(n)`, `Infinite`), with
optional jitter and a dry-run that only counts packets. The set of
injected frame hashes is fed into a shared filter so the capture thread
drops the loopback copy.

**BLE.** Payloads are chunked into BLE notifications with a 4-byte
header (`seq_hi, seq_lo, frag_idx, total_frags`). The per-session chunk
size is negotiated via a 6-byte handshake (max 509 bytes for ATT MTU
512; fallback 244 if the probe is rejected).

**Map view.** MapLibre GL JS embedded in a Dioxus WebView. Three modes:
Demo (synthetic trajectories), Online (live stream), Offline (load a
local pcapng and replay it client-side).

**TTS.** Native engines per platform: `AVSpeechSynthesizer`
(macOS / iOS), WinRT SAPI (Windows), Speech Dispatcher (Linux),
Android TTS. Messages are localized in `TtsMessage::text(lang)` (de/en).

## HTTP API

Default port 8080, CORS permissive.

| Endpoint | Method | Notes |
|---|---|---|
| `/status` | GET | JSON `NodeStatus` (cpu, ram, temp, replay count) |
| `/metrics` | GET | CSV variant of the above (Local only) |
| `/replay/count` | GET | Plain integer (Local only) |
| `/replay/config` | POST | Update `RepeatModeConfig` (Local only) |
| `/pcap/stream` | GET | SSE live PCAP stream (Local only) |
| `/injection/start` | POST | Start with `InjectionConfig` (Local only) |
| `/injection/stop` | POST | Stop current run (Local only) |
| `/injection/pause` | POST | Pause / resume (Local only) |
| `/injection/filter` | POST | Toggle capture filter mid-run (Local only) |
| `/injection/status` | GET | Current `InjectionStatus` (Local only) |
| `/node/config` | GET / POST | Persisted runtime config (log level, port) |
| `/archive/list` | GET | Filenames + packet counts |
| `/archive/latest` | GET | Stream most recent pcapng |
| `/archive/download/{f}` | GET | Stream a specific archive |
| `/archive/upload/{f}` | POST | Upload archive from a client |
| `/tracking/report` | GET | Tracking report (Cloud only) |

## Storage

Plain files in `./captures/`:

- `archive_YYYYMMDD_HHMMSS.pcapng` complete session.
- `incomplete_YYYYMMDD_HHMMSS.pcapng` session with dropped packets.
- `temp_YYYYMMDD_HHMMSS.pcapng` temporary, deleted on clean shutdown.

No database.

## Configuration

`packages/node/config.toml` is the static config. Every field can be
overridden via `CITES_` env vars (e.g.
`CITES_INTERFACES_CAPTURE_INTERFACE=mon0`).

```toml
name = "CITES-Node-3"
mode = "Local"     # or "Cloud"

[interfaces]
capture_interface   = "mon0"   # monitor-mode iface w/ Radiotap
injection_interface = "mon0"   # set to wlan0 (OCB) for real TX
enable_bluetooth    = true
enable_network_api  = true
network_port        = 8080
```

Tracking warning defaults (`TrackingWarningConfig`):

| Field | Default |
|---|---|
| `enabled` | `false` |
| `min_visible_minutes` | `5` |
| `gap_tolerance_secs` | `30` |
| `min_visible_km` | `5.0` |
| `gap_tolerance_km` | `0.5` |

Runtime config (`/var/cites-node/runtime_config.json` on Linux) holds
the log level and API port pushed from the UI; it survives restarts.

## ITS messages

| Protocol | BTP port | Standard |
|---|---|---|
| CAM | 2001 | EN 302 637-2, TS 103 900 |
| DENM | 2002 | EN 302 637-3, TS 103 831 |
| MAPEM | 2003 | IS TS 103 301 |
| SPATEM | 2004 | IS TS 103 301 |
| IVIM | 2006 | IS TS 103 301 |
| SREM | 2007 | IS TS 103 301 |
| SSEM | 2008 | IS TS 103 301 |
| CPM | 2009 | TS 103 324 |
| VAM | 2018 | TS 103 300-3 |

The runtime decoder currently routes CAM and DENM. The other types are
recognized via BTP port but treated as opaque payloads until a decoder
is added.

## Build

Prereqs:

- Rust 2021 or newer
- `libpcap-dev` on Linux, Xcode CLT on macOS
- `cargo install dioxus-cli`
- Docker + Buildx for ARM64 builds

iOS additionally needs a full Xcode plus
`sudo xcodebuild -license` and
`sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`.

Run core tests:

```
cargo test --package core_logic
```

Start desktop app:

```
dx serve --package desktop --platform macos
```

Start web app:

```
dx serve --package web
```

Start iOS app:

```
dx serve --package mobile --platform ios
```

Generate API docs:

```
cargo doc --no-deps
```

To regenerate the ASN.1 C sources before building core:

Make the build script executable:

```
chmod +x packages/core/resources/asn1/build_asn1_c.sh
```

Generate ASN.1 C sources:

```
packages/core/resources/asn1/build_asn1_c.sh
```

Build core library:

```
cargo build --package core_logic
```

## Logging

`tracing` everywhere. Level via `RUST_LOG`.

Desktop logs go to `/tmp/cites-desktop.log`:

```
RUST_LOG=core_logic=debug dx serve --package desktop --platform macos
tail -f /tmp/cites-desktop.log
```

Node (cargo):

```
RUST_LOG=debug cargo run --package node
```

Node (systemd on the Pi) writes to `/var/cites-node/cites-node.log` with
daily rotation. To change the level without redeploying:

```
sudo systemctl edit cites-node
# add under [Service]:
# Environment=RUST_LOG=debug
sudo systemctl restart cites-node
```

At runtime (without restart) the client can push a new level via
`POST /node/config`; the node persists it to `runtime_config.json` so
the next start picks it up too.

## Deployment

`deploy_node.sh` builds an ARM64 binary in Docker, copies it to the Pi
over SSH, and installs the systemd service.

```
./deploy_node.sh                # normal
./deploy_node.sh --force-asn    # also regenerate ASN.1 C sources
```

Two build paths controlled by `USE_REMOTE_DOCKER` at the top of the
script:

- `false` use the local Docker daemon (works natively on Apple Silicon
  via Buildx, no QEMU emulation).
- `true` use a remote x86 host as the Docker daemon. The script will
  generate an SSH key and copy it over if missing.

The script reads `PI_USER`, `PI_HOST`, `PI_BIN_DIR` and `PI_CFG_DIR`
from its top; edit them for your setup.

The installed systemd unit looks like:

```
[Unit]
Description=CITES Node Backend
After=network.target bluetooth.target car2x-startup.service car2x-ocb-setup.service

[Service]
Type=simple
ExecStart=/usr/local/bin/cites-node
WorkingDirectory=/etc/cites
Restart=always
RestartSec=5
User=root
Environment=RUST_LOG=info
```

## Tests

```
cargo test --package core_logic
```

The largest integration test parses a real ITS-G5 capture
(`sample.pcapng`) and diffs the output against a reference
jsonl. Unit tests live in the usual `#[cfg(test)]` blocks.
