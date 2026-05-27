# api

Shared types and helpers that need to compile on the node and on every client.
Kept tiny on purpose so the WASM build does not pull in server-only code.

## What lives here

- `ble_constants.rs` GATT service and characteristic UUIDs, plus the BLE
  command opcodes used over `COMMAND_CHARACTERISTIC`.
- `metrics.rs` `SystemMetrics`, `NodeStatus`, `TrackingReport` and
  `VirtualVehicle`. CSV serialization for the BLE READ path, JSON for HTTP.
- `storage.rs` `PcapStorageManager`: timestamped PCAPNG writer used by the
  node to persist captures and by clients to record their BLE stream.

A single Dioxus server function (`POST /api/metrics`) is exposed for the
fullstack Web build; everything else is plain Rust.

## Notes

Anything that touches sockets, the filesystem on the node side, or capture
hardware belongs in `node`, not here. If you need to add a new server
function, gate its dependencies behind the `server` feature so the WASM
build stays slim.
