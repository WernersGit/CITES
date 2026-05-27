# desktop

Entry point for the desktop client. Wraps the shared `ui` router and
provides a `ConnectionService` to all pages.

## Run it

```
dx serve --package desktop --platform macos
```

(`--platform linux` or `--platform windows` works too; macOS is the one
the project is built on.)

## Logging

Logs go to `/tmp/cites-desktop.log` so they survive a `dx` reload.
Watch them in another shell with:

```
tail -f /tmp/cites-desktop.log
```

The level is controlled via `RUST_LOG`, e.g.
`RUST_LOG=core_logic=debug dx serve --package desktop --platform macos`.

## Routes

`Home`, `SysInfoView`, `AnalysisView`, `ConfigView`, `InjectionView`,
`LiveView`, `FileTransferView`. All defined in `src/main.rs` and rendered
by the shared `Navbar` layout.

## Features

- `desktop` (default): the actual app build.
- `server`: only used when the fullstack build needs SSR.
