# platform

Client-side runtime services that the UI views call into. Everything in
here is shared between desktop, mobile and web but stays out of the
hardware-specific `node` crate.

## What it does

- `ConnectionService` holds all reactive Dioxus signals for the client:
  connection state, PCAP stats, live vehicle list, ego-MAC, TTS queue,
  tracking warning config, injection status. Connection setup over BLE
  (btleplug) and IP (reqwest), plus the reassembly loop that turns BLE
  fragments into parsed ITS packets.
- `tts.rs` runs a small worker thread that speaks queued strings via the
  `tts` crate. The `TtsMessage` enum carries the localized texts (German
  and English).
- `stats.rs` value types used by the UI: `PcapStats`, `LiveVehicleState`,
  `MacTimelinePoint`, plus a helper that derives timeline points from a
  batch of already-parsed packets (used in Offline mode for parity with
  the live stream).

## Dependencies

`btleplug` for BLE, `reqwest` for HTTP, `tts` for speech, `chrono` for
timestamps. No platform-conditional code lives here, just the runtime
glue.
