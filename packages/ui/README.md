# ui

All Dioxus views and shared components. Written once, compiled into the
`desktop`, `mobile` and `web` crates.

## Layout

```
ui/
  src/
    lib.rs            re-exports + localStorage persistence helpers
    navbar.rs         sidebar + top bar
    home.rs           BLE scan, IP connect, connection status
    sysinfo.rs        CPU/RAM/temperature charts
    analysis.rs       per-packet hex and field inspector
    analysis/         supporting charts and pickers
    live.rs           Demo / Online / Offline mode switch
    live/             map view (MapLibre) and car table
    injection.rs      archive picker, filters, schedule, status
    injection/        filter, schedule, packet table, debug panels
    views/config.rs   user configuration view (replay, tracking, TTS, node)
    source_picker.rs  PCAP file picker (rfd)
    trajectory.rs     GeoJSON polyline builder for the map
    file_transfer.rs  archive download/upload page
    countries.rs      ISO 3166 country list for the initial map center
```

## Persistence

`load_persisted_settings()` reads a handful of keys from `localStorage`
and pushes them into the shared `ConnectionService` signal context. The
matching `save_*` helpers return a JS string you can pass to
`document::eval` to write the value back.

## Dependencies

This crate must stay platform-neutral. Put platform-only deps (web_sys,
native dialogs, etc.) into `desktop`, `mobile` or `web` instead.
