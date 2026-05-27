# web

Entry point for the browser client. Same router as desktop and mobile,
but without the `Analysis` page (which depends on a native file dialog).

## Run it

```
dx serve --package web
```

The dev server prints the URL to open.

## How it builds

Because Dioxus fullstack is enabled, the crate is compiled twice:

1. Once for the WASM client (`web` feature, default).
2. Once for the server side (`server` feature) which hosts the
   `POST /api/metrics` server function from `api/`.

Keep web-only deps (anything pulling in `web_sys`, browser APIs) under
the `web` feature so the server build stays small. The native BLE path
does not work in the browser; the home page falls back to the IP input.
