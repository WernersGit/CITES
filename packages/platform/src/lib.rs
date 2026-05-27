pub mod stats;
pub mod tts;
use stats::{PcapStats, MacTimelinePoint, LiveVehicleState};
use dioxus::prelude::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use api::ble_constants;
use api::metrics::NodeStatus;
use api::storage::{PcapStorageManager, StorageMode};
use core_logic::ble_protocol::{BleReassembler, ReassemblyStatus, unpack_pcap_message};
use core_logic::ble_handshake::{
    HandshakeFrame, HANDSHAKE_FRAME_LEN, CLIENT_PROBE_MAX_CHUNK, CLIENT_FALLBACK_CHUNK,
    PROTOCOL_VERSION,
};
use core_logic::config::{RepeatModeConfig, TrackingWarningConfig, NodeConfig, LogLevel};
use core_logic::tracking_warning::{TrackingWarningChecker, WarnReason};
use core_logic::vehicle_tracker::{VehicleTracker, PacketInfo, InsertResult, LAT_LON_SCALE, SPEED_SCALE, HEADING_SCALE};
use core_logic::parser::decoder::ItsPayload;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager};
use uuid::Uuid;
use futures::StreamExt;

/// Command sent to the persistent loop coroutine in the Navbar.
#[derive(Debug, Clone)]
pub enum StartLoopCmd {
    Ip(String),
    Ble,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BleDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Scanning,
    DevicesFound(Vec<BleDevice>),
    Connecting(String),
    ConnectedBT(String),
    ConnectedIP(String),
}

/// Abstract connection service to handle backend connections.
#[derive(Clone, Copy)]
pub struct ConnectionService {
    pub state: Signal<ConnectionState>,
    pub pcap_stats: Signal<PcapStats>,
    pub current_ego_mac: Signal<Option<core_logic::MacStats>>,
    pub ego_mac_status: Signal<String>,
    /// Pending TTS utterances.  The Car2X view drains this queue and speaks each entry.
    pub tts_queue: Signal<Vec<String>>,
    /// How long a foreign MAC must be silent before "Fremdfahrzeug verloren" fires (ms).
    pub foreign_vehicle_timeout_ms: Signal<u64>,
    /// MAC presence timeline for the scatter chart.  1-second resolution per MAC,
    /// capped at 6 000 points (≈ 10 min × 10 MACs).  Written every 500 ms.
    pub mac_timeline: Signal<Vec<MacTimelinePoint>>,
    /// Base URL of the tileserver-gl instance for all MapLibre map views (no trailing slash).
    pub live_tile_server_url: Signal<String>,
    /// ISO 3166-1 alpha-2 country code used as the initial map center (e.g. "DE").
    pub country_code: Signal<String>,
    /// Active tracking-warning configuration; read by the reassembly loop every 200 ms.
    pub tracking_warning_cfg: Signal<TrackingWarningConfig>,
    /// Whether TTS announcements are enabled.  Read by the Navbar drain coroutine.
    pub tts_enabled: Signal<bool>,
    /// BCP 47 language tag used for TTS voice selection (e.g. "de", "en", "fr").
    /// Empty string means system default.
    pub tts_language: Signal<String>,
    /// Node-level settings (log level, API port) pushed to the connected node.
    pub node_cfg: Signal<NodeConfig>,
    /// Set to `Some(node_level)` when the just-connected node's log level differs
    /// from the client's configured level.  Cleared after the user resolves it.
    pub log_level_conflict: Signal<Option<LogLevel>>,
    /// Live vehicle states updated by the reassembly loop (Online mode).
    pub live_vehicles: Signal<Vec<LiveVehicleState>>,
    /// Path of the pcapng file being written by the current capture session.
    /// Set when a capture loop starts, cleared when it ends.
    /// The frontend polls this file to derive trajectories and vehicle IDs.
    pub current_capture_path: Signal<Option<String>>,
    /// Current injection engine status, polled by the persistent Navbar coroutine.
    pub inj_status: Signal<core_logic::config::InjectionStatus>,
    /// Set to `true` while an injection run is active; clears automatically when done.
    pub inj_polling: Signal<bool>,
    /// User-controlled capture-filter toggle for the injection run. Sticky during
    /// a run, reset to `true` by the Navbar coroutine when the run ends.
    pub filter_inj: Signal<bool>,
    /// User-controlled "preserve original inter-packet timing" toggle. Sticky
    /// during a run, reset to `true` when the run ends.
    pub pres_timing: Signal<bool>,
    /// Handle to the persistent loop coroutine living in Navbar.
    /// Set by Navbar after it creates the coroutine; used by connect_* methods to
    /// send StartLoopCmd without spawning a scope-bound task.
    pub loop_coroutine: Signal<Option<Coroutine<StartLoopCmd>>>,
    /// Display format for the offline-mode packet timestamp on the Live page.
    /// `false` = human-readable (hh:mm:ss dd.mm.yyyy), `true` = Unix ms integer.
    pub ts_unix_format: Signal<bool>,
    /// Timestamp source for the offline-mode display on the Live page.
    /// `false` = recording timestamp (PCAP capture time), `true` = GNW LPV TST.
    pub ts_use_gnw: Signal<bool>,
    /// Incremented on every new connection; lets stale reassembly loops detect that
    /// a newer session has started and self-terminate.
    pcap_session_my_session: Signal<u32>,
    adapter: Signal<Option<Arc<Adapter>>>,
    active_peripheral: Signal<Option<btleplug::platform::Peripheral>>,
}

impl ConnectionService {
    pub fn new() -> Self {
        Self {
            state: Signal::new(ConnectionState::Disconnected),
            pcap_stats: Signal::new(PcapStats::default()),
            current_ego_mac: Signal::new(None),
            ego_mac_status: Signal::new("Initializing".to_string()),
            tts_queue: Signal::new(Vec::new()),
            foreign_vehicle_timeout_ms: Signal::new(1000),
            mac_timeline: Signal::new(Vec::new()),
            live_tile_server_url: Signal::new("http://localhost:8080".to_string()),
            country_code: Signal::new("DE".to_string()),
            tracking_warning_cfg: Signal::new(TrackingWarningConfig::default()),
            tts_enabled: Signal::new(false),
            tts_language: Signal::new(String::new()),
            node_cfg: Signal::new(NodeConfig::default()),
            log_level_conflict: Signal::new(None),
            live_vehicles: Signal::new(Vec::new()),
            current_capture_path: Signal::new(None),
            inj_status: Signal::new(core_logic::config::InjectionStatus::default()),
            inj_polling: Signal::new(false),
            filter_inj: Signal::new(true),
            pres_timing: Signal::new(true),
            loop_coroutine: Signal::new(None),
            ts_unix_format: Signal::new(false),
            ts_use_gnw: Signal::new(false),
            pcap_session_my_session: Signal::new(0),
            adapter: Signal::new(None),
            active_peripheral: Signal::new(None),
        }
    }

    /// Resolves `msg` to a localised string using `tts_language` and enqueues it.
    pub fn announce(&mut self, msg: tts::TtsMessage) {
        let lang = self.tts_language.read().clone();
        self.tts_queue.write().push(msg.text(&lang));
    }

    async fn init_adapter(&mut self) -> Option<Arc<Adapter>> {
        if let Some(adapter) = self.adapter.read().clone() {
            return Some(adapter);
        }

        let manager = match Manager::new().await {
            Ok(m) => m,
            Err(_) => return None,
        };

        let adapters = match manager.adapters().await {
            Ok(a) => a,
            Err(_) => return None,
        };

        // TODO: handle multiple adapters at some point
        let a = adapters.into_iter().next()?;
        let shared = Arc::new(a);
        *self.adapter.write() = Some(shared.clone());
        Some(shared)
    }

    pub async fn start_bluetooth_scan(&mut self) {
        *self.state.write() = ConnectionState::Scanning;

        let adapter = match self.init_adapter().await {
            Some(a) => a,
            None => {
                *self.state.write() = ConnectionState::Disconnected;
                return;
            }
        };

        // filter at the OS/hardware level so only our service UUID is tracked.
        // without this, the adapter collects every nearby BLE device and relies on a post-scan name check - advertisement packets can easily be missed
        let svc_uuid = Uuid::parse_str(ble_constants::CITES_SERVICE_UUID).unwrap();
        let scan_filter = ScanFilter { services: vec![svc_uuid] };

        if adapter.start_scan(scan_filter).await.is_err() {
            *self.state.write() = ConnectionState::Disconnected;
            return;
        }

        // poll every 500 ms for up to 15 s; exit as soon as the node appears
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            async_std::task::sleep(Duration::from_millis(500)).await;

            let found = Self::collect_cites_devices(&adapter).await;
            if !found.is_empty() {
                let _ = adapter.stop_scan().await;
                *self.state.write() = ConnectionState::DevicesFound(found);
                return;
            }

            if Instant::now() >= deadline {
                break;
            }
        }

        let _ = adapter.stop_scan().await;
        *self.state.write() = ConnectionState::Disconnected;
    }

    async fn collect_cites_devices(adapter: &Adapter) -> Vec<BleDevice> {
        let peripherals = adapter.peripherals().await.unwrap_or_default();
        let mut found = Vec::new();
        for p in peripherals {
            if let Ok(Some(props)) = p.properties().await {
                let has_uuid = props.services.iter()
                    .any(|u| u.to_string() == ble_constants::CITES_SERVICE_UUID);
                let has_name = props.local_name.as_ref()
                    .map(|n| n.starts_with(ble_constants::CITES_MAC_NAME_PREFIX))
                    .unwrap_or(false);
                if has_uuid || has_name {
                    found.push(BleDevice {
                        id: p.id().to_string(),
                        name: props.local_name.unwrap_or_else(|| "Unknown CITES Node".into()),
                    });
                }
            }
        }
        found
    }

    /// negotiates MTU and performs the BLE session handshake
    // TODO: expose negotiated chunk size for diagnostics
    async fn perform_ble_handshake(
        peripheral: &btleplug::platform::Peripheral,
    ) -> Result<HandshakeFrame, String> {
        let target = Uuid::parse_str(ble_constants::HANDSHAKE_CHARACTERISTIC_UUID)
            .map_err(|e| e.to_string())?;
        let chars = peripheral.characteristics();
        let ch = chars.iter().find(|c| c.uuid == target)
            .ok_or_else(|| "HANDSHAKE_CHARACTERISTIC not found".to_string())?;

        // step 1: probe write (WithoutResponse, CLIENT_PROBE_MAX_CHUNK bytes)
        let req_frame = HandshakeFrame {
            version: PROTOCOL_VERSION,
            max_chunk: CLIENT_PROBE_MAX_CHUNK,
            capabilities: 0,
        };
        let mut probe = vec![0u8; CLIENT_PROBE_MAX_CHUNK as usize];
        probe[..HANDSHAKE_FRAME_LEN].copy_from_slice(&req_frame.to_bytes());

        let accepted = peripheral
            .write(ch, &probe, WriteType::WithoutResponse)
            .await
            .is_ok();

        // writeWithoutResponse returns as soon as data is queued — not when the server has processed it.
        // Wait one BLE round-trip + BlueZ handler latency before reading the confirmation back
        if accepted {
            async_std::task::sleep(Duration::from_millis(50)).await;
        }

        // step 2: READ confirmation — the server tells us what it received
        let raw = peripheral.read(ch).await
            .map_err(|e| format!("handshake read failed: {e}"))?;
        let confirmed = HandshakeFrame::from_bytes(&raw)
            .ok_or_else(|| "handshake response malformed".to_string())?;

        // If the probe was locally rejected OR the server did not confirm the probed max (probe was silently dropped),
        // send a corrective write so both sides agree on the fallback value
        if !accepted || confirmed.max_chunk < CLIENT_PROBE_MAX_CHUNK {
            let fb = HandshakeFrame {
                version: PROTOCOL_VERSION,
                max_chunk: CLIENT_FALLBACK_CHUNK,
                capabilities: 0,
            };
            peripheral
                .write(ch, &fb.to_bytes(), WriteType::WithResponse)
                .await
                .map_err(|e| format!("handshake fallback write failed: {e}"))?;

            let raw2 = peripheral.read(ch).await
                .map_err(|e| format!("handshake fallback read failed: {e}"))?;
            let final_frame = HandshakeFrame::from_bytes(&raw2)
                .ok_or_else(|| "handshake fallback response malformed".to_string())?;

            tracing::info!(
                "BLE handshake: probe {}B {} -> fallback {}B negotiated",
                CLIENT_PROBE_MAX_CHUNK,
                if accepted { "unconfirmed by server" } else { "rejected by OS (ATT_MTU too small)" },
                final_frame.max_chunk,
            );
            return Ok(final_frame);
        }

        tracing::info!(
            "BLE handshake: probe {}B confirmed -> negotiated {}B",
            CLIENT_PROBE_MAX_CHUNK,
            confirmed.max_chunk,
        );
        Ok(confirmed)
    }

    pub async fn connect_to_device(&mut self, device_id: String, device_name: String) {
        *self.state.write() = ConnectionState::Connecting(device_name.clone());

        let adapter = if let Some(a) = self.adapter.read().clone() {
            a
        } else {
            *self.state.write() = ConnectionState::Disconnected;
            return;
        };

        if let Ok(peripherals) = adapter.peripherals().await {
            for p in peripherals {
                if p.id().to_string() == device_id {
                    let _ = p.connect().await;
                    let _ = p.discover_services().await;

                    // Negotiate the session chunk size before subscribing to pcap notifications so the very first notification fits the
                    // OS-negotiated ATT_MTU. Failures are logged but non-fatal -> the server initialises with a safe default.
                    match Self::perform_ble_handshake(&p).await {
                        Ok(frame) => tracing::info!(
                            "BLE handshake ok: negotiated v{} max_chunk={}B",
                            frame.version, frame.max_chunk,
                        ),
                        Err(e) => tracing::warn!("BLE handshake skipped: {e}"),
                    }

                    // bump session so any stale reassembly loop self-terminates
                    self.pcap_session_my_session.with_mut(|g| *g = g.wrapping_add(1));
                    *self.active_peripheral.write() = Some(p);
                    *self.state.write() = ConnectionState::ConnectedBT(device_name.clone());

                    // delegate loop to the persistent Navbar coroutine
                    match self.loop_coroutine.read().clone() {
                        Some(co) => co.send(StartLoopCmd::Ble),
                        None => tracing::error!("connect_to_device: loop_coroutine not set"),
                    }
                    return;
                }
            }
        }

        *self.state.write() = ConnectionState::Disconnected;
    }

    pub async fn connect_ip(&mut self, ip: &str) -> Result<(), String> {
        *self.state.write() = ConnectionState::Connecting(ip.to_string());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;

        let url = format!("http://{}:8080/status", ip);
        let resp = client.get(&url).send().await.map_err(|e| {
            *self.state.write() = ConnectionState::Disconnected;
            if e.is_timeout() {
                format!("Timeout: {ip} did not respond within 5 s")
            } else if e.is_connect() {
                format!("Connection refused: {ip} port 8080 not open")
            } else {
                format!("Cannot reach {}: {}", ip, e)
            }
        })?;

        if !resp.status().is_success() {
            *self.state.write() = ConnectionState::Disconnected;
            return Err(format!(
                "{} returned HTTP {} — not a CITES node",
                ip,
                resp.status()
            ));
        }

        let _status = resp.json::<NodeStatus>().await.map_err(|_| {
            *self.state.write() = ConnectionState::Disconnected;
            format!("{} is reachable but not a CITES node (invalid /status response)", ip)
        })?;

        self.pcap_session_my_session.with_mut(|g| *g = g.wrapping_add(1));
        *self.state.write() = ConnectionState::ConnectedIP(ip.to_string());

        // delegate SSE loop to the persistent Navbar coroutine
        if let Some(co) = self.loop_coroutine.read().clone() {
            co.send(StartLoopCmd::Ip(ip.to_string()));
        }

        Ok(())
    }

    /// Subscribes to the stored BLE peripheral's PCAP characteristic and runs the reassembly loop until the stream ends.
    /// called exclusively from the persistent Navbar loop coroutine
    pub async fn start_ble_pcap_loop(&mut self) {
        let peripheral = match self.active_peripheral.read().clone() {
            Some(p) => p,
            None => {
                tracing::warn!("start_ble_pcap_loop: no active peripheral");
                return;
            }
        };

        let pcap_uuid = Uuid::parse_str(ble_constants::PCAP_CHARACTERISTIC_UUID)
            .unwrap_or_default();
        let chars = peripheral.characteristics();
        tracing::info!("BLE PCAP loop: discovered {} characterisitcs", chars.len());

        let pcap_char = match chars.iter().find(|c| c.uuid == pcap_uuid) {
            Some(c) => c.clone(),
            None => {
                tracing::error!("BLE PCAP loop: PCAP characteristic missing — re-pair the device");
                return;
            }
        };

        if let Err(e) = peripheral.subscribe(&pcap_char).await {
            tracing::error!("BLE PCAP loop: subscribe failed: {e}");
            return;
        }

        let my_session = *self.pcap_session_my_session.read();
        match peripheral.notifications().await {
            Err(e) => tracing::error!("BLE PCAP loop: notifications() failed: {e}"),
            Ok(notif_stream) => {
                tracing::info!("BLE PCAP loop: notifcation stream opened (session {my_session}), starting reassembly");
                let pcap_stream = Box::pin(
                    notif_stream
                        .filter(move |n| futures::future::ready(n.uuid == pcap_uuid))
                        .map(|n| n.value),
                );
                self.start_pcap_reassembly_loop(pcap_stream, StorageMode::Temporary, my_session).await;
                tracing::info!("BLE PCAP loop: reassmebly loop exited");
            }
        }
    }

    /// Opens the SSE stream for an IP-connected node and runs the reassembly
    /// loop until the stream ends.
    /// Called exclusively from the persistent Navbar loop coroutine.
    pub async fn start_ip_pcap_loop(&mut self, ip: &str) {
        let my_session = *self.pcap_session_my_session.read();
        let stream_url = format!("http://{}:8080/pcap/stream", ip);
        match reqwest::get(&stream_url).await {
            Ok(resp) => {
                self.start_pcap_reassembly_loop(
                    sse_to_fragment_stream(resp),
                    StorageMode::Temporary,
                    my_session,
                ).await;
            }
            Err(e) => tracing::error!("start_ip_pcap_loop: GET failed: {e}"),
        }
    }

    /// Fetches the unified node status (metrics + replay counter).
    ///
    /// - **IP mode**: `GET /status` → JSON [`NodeStatus`]
    /// - **BLE mode**: READ `METRICS_CHARACTERISTIC` → CSV parsed as [`NodeStatus`]
    ///   (replay_count is the last CSV field)
    pub async fn fetch_status(&self) -> Result<NodeStatus, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/status", ip);
                let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
                resp.json::<NodeStatus>().await.map_err(|e| e.to_string())
            }
            ConnectionState::ConnectedBT(_) => {
                if let Some(peripheral) = self.active_peripheral.read().clone() {
                    let chars = peripheral.characteristics();
                    let target: uuid::Uuid = std::str::FromStr::from_str(
                        ble_constants::METRICS_CHARACTERISTIC_UUID
                    ).unwrap_or_default();
                    if let Some(c) = chars.iter().find(|c| c.uuid == target) {
                        let bytes = peripheral.read(c).await.map_err(|e| e.to_string())?;
                        let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
                        NodeStatus::from_csv(&text)
                    } else {
                        Err("METRICS_CHARACTERISTIC not found".into())
                    }
                } else {
                    Err("No active BLE peripheral".into())
                }
            }
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Fetches legacy [`SystemMetrics`] by delegating to [`fetch_status`].
    /// Kept for backward compatibility; prefer `fetch_status` in new code.
    pub async fn fetch_metrics(&self) -> Result<api::SystemMetrics, String> {
        let s = self.fetch_status().await?;
        Ok(api::SystemMetrics {
            cpu_usage: s.cpu_usage,
            ram_used_mb: s.ram_used_mb,
            ram_total_mb: s.ram_total_mb,
            temp_celsius: s.temp_celsius,
            mac_address: "FF:FF:FF:FF:FF:FF".to_string(),
        })
    }

    /// Pushes a [`RepeatModeConfig`] to the connected node.
    ///
    /// - **IP mode**: `POST /replay/config` with JSON body
    /// - **BLE mode**: WRITE `COMMAND_CHARACTERISTIC` with 10-byte binary frame
    ///   (see [`RepeatModeConfig::to_ble_binary`])
    pub async fn push_replay_config(&self, cfg: RepeatModeConfig) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/replay/config", ip);
                reqwest::Client::new()
                    .post(&url)
                    .json(&cfg)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            ConnectionState::ConnectedBT(_) => {
                if let Some(peripheral) = self.active_peripheral.read().clone() {
                    let chars = peripheral.characteristics();
                    let target: uuid::Uuid = std::str::FromStr::from_str(
                        ble_constants::COMMAND_CHARACTERISTIC_UUID
                    ).unwrap_or_default();
                    if let Some(c) = chars.iter().find(|c| c.uuid == target) {
                        peripheral
                            .write(c, &cfg.to_ble_binary(), WriteType::WithoutResponse)
                            .await
                            .map_err(|e| e.to_string())
                    } else {
                        Err("COMMAND_CHARACTERISTIC not found".into())
                    }
                } else {
                    Err("No active BLE peripheral".into())
                }
            }
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Pushes a [`NodeConfig`] (log level + API port) to the connected node.
    ///
    /// - **IP mode**: `POST /node/config` with JSON body
    /// - **BLE mode**: WRITE `COMMAND_CHARACTERISTIC` with 5-byte binary frame
    pub async fn push_node_config(&self, cfg: NodeConfig) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/node/config", ip);
                reqwest::Client::new()
                    .post(&url)
                    .json(&cfg)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            ConnectionState::ConnectedBT(_) =>
                self.ble_write_command(&cfg.to_ble_binary()).await,
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Reads the current [`NodeConfig`] from the connected node.
    ///
    /// - **IP mode**: `GET /node/config` → JSON
    /// - **BLE mode**: READ `NODE_CONFIG_CHARACTERISTIC` → 3-byte status frame
    pub async fn fetch_node_config(&self) -> Result<NodeConfig, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/node/config", ip);
                let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
                resp.json::<NodeConfig>().await.map_err(|e| e.to_string())
            }
            ConnectionState::ConnectedBT(_) => {
                let peripheral = self.active_peripheral.read().clone()
                    .ok_or_else(|| "No active BLE peripheral".to_string())?;
                let chars = peripheral.characteristics();
                let uuid = uuid::Uuid::parse_str(ble_constants::NODE_CONFIG_CHARACTERISTIC_UUID)
                    .map_err(|e| e.to_string())?;
                let c = chars.iter().find(|c| c.uuid == uuid)
                    .ok_or_else(|| "NODE_CONFIG_CHARACTERISTIC not found".to_string())?;
                let data = peripheral.read(c).await.map_err(|e| e.to_string())?;
                NodeConfig::from_ble_status(&data)
                    .ok_or_else(|| "Failed to decode node config".to_string())
            }
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Fetches the cumulative replay counter.  Calls [`fetch_status`] internally so no additional request is made when status is already being polled.
    pub async fn fetch_replay_count(&self) -> Result<u64, String> {
        self.fetch_status().await.map(|s| s.replay_count)
    }

    /// Fetches archive filenames from the connected node.
    ///
    /// - IP: JSON array from `GET /archive/list`.
    /// - BLE: reads `ARCHIVE_LIST_CHARACTERISTIC`; the payload is newline-separated
    ///   `filename:count` lines — only the filenames are returned here.
    ///   Call [`fetch_archive_packet_counts`] to get the counts.
    pub async fn fetch_archive_list(&self) -> Result<Vec<String>, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/archive/list", ip);
                let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
                resp.json::<Vec<String>>().await.map_err(|e| e.to_string())
            }
            ConnectionState::ConnectedBT(_) => {
                let entries = self.ble_read_archive_list().await?;
                Ok(entries.into_iter().map(|(name, _)| name).collect())
            }
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Returns `(filename, valid_packet_count)` pairs from the BLE archive list
    /// characteristic.  Returns an empty vec for IP connections (counts are not
    /// pre-computed server-side for HTTP).
    pub async fn fetch_archive_packet_counts(&self) -> Result<Vec<(String, u64)>, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedBT(_) => self.ble_read_archive_list().await,
            _ => Ok(Vec::new()),
        }
    }

    async fn ble_read_archive_list(&self) -> Result<Vec<(String, u64)>, String> {
        let peripheral = self.active_peripheral.read().clone()
            .ok_or_else(|| "No active BLE peripheral".to_string())?;
        let chars = peripheral.characteristics();
        let uuid = uuid::Uuid::parse_str(ble_constants::ARCHIVE_LIST_CHARACTERISTIC_UUID)
            .map_err(|e| e.to_string())?;
        let c = chars.iter().find(|c| c.uuid == uuid)
            .ok_or_else(|| "ARCHIVE_LIST_CHARACTERISTIC not found".to_string())?;
        let data = peripheral.read(c).await.map_err(|e| e.to_string())?;
        let text = String::from_utf8(data).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for line in text.lines() {
            if let Some((name, count)) = line.rsplit_once(':') {
                let n = count.parse::<u64>().unwrap_or(0);
                entries.push((name.to_string(), n));
            }
        }
        Ok(entries)
    }

    /// Uploads a PCAPNG file to the connected node
    /// `filename` must be a plain filename (no path separators)
    pub async fn upload_archive_file(&self, filename: &str, data: Vec<u8>) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/archive/upload/{}", ip, filename);
                let resp = reqwest::Client::new()
                    .post(&url)
                    .body(data)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if resp.status().is_success() {
                    Ok(())
                } else {
                    Err(format!("HTTP {}", resp.status()))
                }
            }
            _ => Err("Upload requires an IP connection.".into()),
        }
    }

    /// Downloads a specific PCAPNG archive file from the connected node
    /// `filename` must be a plain filename (no path separators)
    pub async fn fetch_archive_file(&self, filename: &str) -> Result<Vec<u8>, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/archive/download/{}", ip, filename);
                let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
                if !resp.status().is_success() {
                    return Err(format!("HTTP {}", resp.status()));
                }
                resp.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
            }
            _ => Err("Archive download is only available via IP connection.".into()),
        }
    }

    /// Starts a new injection run on the connected node
    /// Stops any previously running injection first
    pub async fn start_injection(
        &self,
        config: core_logic::config::InjectionConfig,
    ) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/injection/start", ip);
                reqwest::Client::new()
                    .post(&url)
                    .json(&config)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            ConnectionState::ConnectedBT(_) =>
                self.ble_write_command(&config.to_ble_binary()).await,
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Signals the node to stop the active injection run.
    pub async fn stop_injection(&self) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                reqwest::Client::new()
                    .post(format!("http://{}:8080/injection/stop", ip))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            ConnectionState::ConnectedBT(_) =>
                self.ble_write_command(&[ble_constants::BLE_CMD_MAGIC, ble_constants::BLE_CMD_STOP_INJECTION]).await,
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Live-updates the node's capture-filter toggle for the active injection.
    ///
    /// Takes effect on the next packet. Reflected back via `InjectionStatus.filter_inj`.
    pub async fn set_inj_filter(&self, on: bool) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                reqwest::Client::new()
                    .post(format!("http://{}:8080/injection/filter", ip))
                    .json(&on)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            ConnectionState::ConnectedBT(_) => {
                let payload = [
                    ble_constants::BLE_CMD_MAGIC,
                    ble_constants::BLE_CMD_SET_INJ_FILTER,
                    if on { 1 } else { 0 },
                ];
                self.ble_write_command(&payload).await
            }
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Toggles the pause state of the active injection run.
    pub async fn pause_injection(&self) -> Result<(), String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                reqwest::Client::new()
                    .post(format!("http://{}:8080/injection/pause", ip))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            ConnectionState::ConnectedBT(_) =>
                self.ble_write_command(&[ble_constants::BLE_CMD_MAGIC, ble_constants::BLE_CMD_PAUSE_INJECTION]).await,
            _ => Err("Not connected to any node.".into()),
        }
    }

    /// Fetches the current injection engine status from the node.
    pub async fn fetch_injection_status(
        &self,
    ) -> Result<core_logic::config::InjectionStatus, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/injection/status", ip);
                reqwest::get(&url)
                    .await
                    .map_err(|e| e.to_string())?
                    .json::<core_logic::config::InjectionStatus>()
                    .await
                    .map_err(|e| e.to_string())
            }
            ConnectionState::ConnectedBT(_) => {
                let peripheral = self.active_peripheral.read().clone()
                    .ok_or_else(|| "No active BLE peripheral".to_string())?;
                let chars = peripheral.characteristics();
                let uuid = uuid::Uuid::parse_str(ble_constants::INJECTION_STATUS_CHARACTERISTIC_UUID)
                    .map_err(|e| e.to_string())?;
                let c = chars.iter().find(|c| c.uuid == uuid)
                    .ok_or_else(|| "INJECTION_STATUS_CHARACTERISTIC not found".to_string())?;
                let data = peripheral.read(c).await.map_err(|e| e.to_string())?;
                core_logic::config::InjectionStatus::from_ble_binary(&data)
                    .ok_or_else(|| "Failed to decode injection status".to_string())
            }
            _ => Err("Not connected to any node.".into()),
        }
    }

    async fn ble_write_command(&self, payload: &[u8]) -> Result<(), String> {
        let peripheral = self.active_peripheral.read().clone()
            .ok_or_else(|| "No active BLE peripheral".to_string())?;
        let chars = peripheral.characteristics();
        let uuid = uuid::Uuid::parse_str(ble_constants::COMMAND_CHARACTERISTIC_UUID)
            .map_err(|e| e.to_string())?;
        let c = chars.iter().find(|c| c.uuid == uuid)
            .ok_or_else(|| "COMMAND_CHARACTERISTIC not found".to_string())?;
        peripheral.write(c, payload, WriteType::WithoutResponse)
            .await.map_err(|e| e.to_string())
    }

    /// Requests an on-demand tracking report from a Cloud node.
    ///
    /// Only meaningful for IP connections.  Returns an error for BLE connections (Cloud nodes never expose a BLE interface) and when not connected.
    pub async fn fetch_tracking_report(&self) -> Result<api::TrackingReport, String> {
        match self.state.read().clone() {
            ConnectionState::ConnectedIP(ip) => {
                let url = format!("http://{}:8080/tracking/report", ip);
                let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
                resp.json::<api::TrackingReport>().await.map_err(|e| e.to_string())
            }
            _ => Err("Tracking reports are only available from Cloud nodes via IP.".into()),
        }
    }

    pub fn disconnect(&mut self) {
        if let Some(peripheral) = self.active_peripheral.write().take() {
            async_std::task::spawn(async move {
                let _ = peripheral.disconnect().await;
            });
        }
        *self.state.write() = ConnectionState::Disconnected;
    }

    /// BLE reassembly and live stats loop.
    ///
    /// Changes vs. previous version:
    /// - `pcap_stats` is written at most every **100 ms** to avoid 50 Hz signal-writes flooding the Dioxus render queue.
    /// - Gap detection: after each successful reassembly the full u64 sequence
    ///   number is extracted from the packed payload and compared with the previous one.  Missing packets are counted in `PcapStats::missed_packets`.
    pub async fn start_pcap_reassembly_loop(
        &mut self,
        mut stream: impl futures::Stream<Item = Vec<u8>> + Unpin,
        storage_mode: StorageMode,
        my_session: u32,
    ) {
        tracing::info!("reassembly lopo started (session {my_session})");
        let mut storage = match PcapStorageManager::new(storage_mode, "./client_captures") {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("stoarge init failed: {e}");
                return;
            }
        };
        self.current_capture_path.set(Some(storage.filepath.to_string_lossy().into_owned()));
        *self.live_vehicles.write() = Vec::new();

        let mut reassembler = BleReassembler::new();
        let mut current_stats = PcapStats::default();
        let mut ego_analyzer = core_logic::ego_mac::EgoMac::new(10_000, 5);
        let mut tracker = VehicleTracker::new();
        let mut checker = TrackingWarningChecker::new(
            self.tracking_warning_cfg.read().clone(),
        );

        // for gap detection we track the last authoritative u64 seq number
        let mut last_seq: Option<u64> = None;

        // TTS: track the last spoken ego MAC to detect changes
        let mut last_spoken_ego_mac: Option<String> = None;

        // throttle: only write the stats signal at most every 100 ms
        let mut stats_ts = Instant::now();
        const STATS_INTERVAL: Duration = Duration::from_millis(100);

        // timeuot check: run at most every 200 ms to avoid busy-spinning
        let mut timeout_ts = Instant::now();
        const TIMEOUT_CHECK_INTERVAL: Duration = Duration::from_millis(200);

        // timeline accumulation: batch-write new presence points every 500 ms.
        // at most one point per MAC per 1 000 ms bucket (1-second resolution)
        let mut pending: Vec<MacTimelinePoint> = Vec::new();
        let mut last_mac_bucket: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        let mut tl_ts = Instant::now();
        const TIMELINE_WRITE_INTERVAL: Duration = Duration::from_millis(500);
        const TIMELINE_BUCKET_MS: i64 = 1_000;
        const TIMELINE_MAX_POINTS: usize = 6_000;

        // live vehicle state map: keyed by virtual_id
        let mut veh_map: std::collections::HashMap<u32, LiveVehicleState> = std::collections::HashMap::new();
        let mut live_ts = Instant::now();
        const LIVE_WRITE_INTERVAL: Duration = Duration::from_millis(500);
        // 30-second keepalive timeout: exits the loop if the connection drops but btleplug does not close the notification stream (macOS Core Bluetooth
        // timing issue that would otherwise block the Dioxus coroutine forever)
        'recv: loop {
            let chunk = match async_std::future::timeout(
                Duration::from_secs(30),
                stream.next(),
            ).await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(_) => {
                    if *self.pcap_session_my_session.read() != my_session {
                        tracing::info!("reassembly loop: superseded by newer session, exiting");
                        break;
                    }
                    continue 'recv;
                }
            };
            let (past_status, current_status) = reassembler.process_chunk(&chunk);

            if past_status == ReassemblyStatus::Dropped {
                storage.mark_incomplete();
                current_stats.is_incomplete = true;
                current_stats.dropped_fragments += 1;
            }

            match current_status {
                ReassemblyStatus::Complete(_header_seq, payload) => {
                    if let Some((seq, ts_ns, data)) = unpack_pcap_message(&payload) {
                        // gap detection using the authoritative archive sequence number
                        if let Some(prev) = last_seq {
                            let expected = prev.wrapping_add(1);
                            if seq != expected && seq > expected {
                                current_stats.missed_packets += seq - expected;
                            }
                        }
                        last_seq = Some(seq);

                        let bytes_len = data.len() as u64;
                        if storage.write_packet(ts_ns, data).is_err() {
                            current_stats.is_incomplete = true;
                        } else {
                            current_stats.total_packets += 1;
                            current_stats.total_bytes += bytes_len;

                            if let Some(parsed) = core_logic::pcap_parser::PcapParser::parse_live_packet(ts_ns, data) {
                                ego_analyzer.insert_measurement(
                                    parsed.timestamp_ms,
                                    parsed.mac.clone(),
                                    parsed.rssi,
                                );

                                // timeline: add at most one presence point per MAC per second
                                let bucket = parsed.timestamp_ms / TIMELINE_BUCKET_MS;
                                let last = last_mac_bucket.get(&parsed.mac).copied().unwrap_or(-1);
                                if bucket != last {
                                    last_mac_bucket.insert(parsed.mac.clone(), bucket);
                                    pending.push(MacTimelinePoint {
                                        timestamp_ms: bucket * TIMELINE_BUCKET_MS,
                                        mac: parsed.mac.clone(),
                                    });
                                }

                                // build PacketInfo for the vehicle tracker
                                let cam = parsed.payload.as_ref().and_then(|p| {
                                    if let ItsPayload::Cam(cam) = p { Some(cam.as_ref()) } else { None }
                                });
                                let pkt_info = PacketInfo {
                                    mac: parsed.mac.clone(),
                                    timestamp_ms: parsed.timestamp_ms,
                                    lat: parsed.gnw_info.as_ref().map(|g| g.latitude as f64 * LAT_LON_SCALE),
                                    lon: parsed.gnw_info.as_ref().map(|g| g.longitude as f64 * LAT_LON_SCALE),
                                    pos_confidence_m:  cam.and_then(|c| c.pos_confidence_m),
                                    speed_kmh: parsed.gnw_info.as_ref().map(|g| g.speed as f64 * SPEED_SCALE),
                                    spd_conf:          cam.and_then(|c| c.speed_confidence_ms),
                                    heading_deg: parsed.gnw_info.as_ref().map(|g| g.heading as f64 * HEADING_SCALE),
                                    hdg_conf:          cam.and_then(|c| c.heading_confidence_deg),
                                    yaw_rate:          cam.and_then(|c| c.yaw_rate),
                                    yaw_conf:          cam.and_then(|c| c.yaw_rate_confidence_deg_s),
                                    accel:             cam.and_then(|c| c.longitudinal_accel),
                                    brake: cam.and_then(|c| c.accel_control.as_ref()).map(|a| a.brake_pedal_active),
                                    gas:   cam.and_then(|c| c.accel_control.as_ref()).map(|a| a.gas_pedal_active),
                                    curvature:  cam.and_then(|c| c.curvature),
                                    v_len:      cam.and_then(|c| c.vehicle_length_m),
                                    v_wid:      cam.and_then(|c| c.vehicle_width_m),
                                    frame_seq:  parsed.frame_seq,
                                };

                                // re-evaluate ego identity every 10 packets
                                if current_stats.total_packets % 10 == 0 {
                                    if let Some(top_mac) = ego_analyzer.evaluate().first().cloned() {
                                        tracker.set_ego_macs(
                                            ego_analyzer.get_timeline()
                                                .iter()
                                                .map(|m| m.mac.clone())
                                        );

                                        // ego MAC changed -> speak the new MAC
                                        if last_spoken_ego_mac.as_deref() != Some(top_mac.mac.as_str()) {
                                            last_spoken_ego_mac = Some(top_mac.mac.clone());
                                            let spoken = tts::mac_to_spoken(&top_mac.mac);
                                            self.announce(tts::TtsMessage::EgoMac(spoken));
                                            *self.current_ego_mac.write() = Some(top_mac);
                                            *self.ego_mac_status.write() = "Active Evaluation".to_string();
                                        }
                                    } else {
                                        *self.ego_mac_status.write() = "Waiting for data...".to_string();
                                    }
                                }

                                // track foreign vehicles
                                let insert_result = tracker.insert_packet(pkt_info);
                                let is_ego = matches!(insert_result, InsertResult::Ego);
                                match insert_result {
                                    InsertResult::NewVehicle(_) => {
                                        self.announce(tts::TtsMessage::ForeignVehicleFound);
                                    }
                                    InsertResult::Ego | InsertResult::Known => {}
                                }

                                // update live vehicle state map
                                if let Some(vid) = tracker.get_vid_for_mac(&parsed.mac) {
                                    // all ego MACs share virtual_id 0 so only one ego entry appears
                                    let effective_vid = if is_ego { 0 } else { vid };
                                    let entry = veh_map.entry(effective_vid).or_insert_with(|| LiveVehicleState {
                                        virtual_id: effective_vid,
                                        ..Default::default()
                                    });
                                    if !entry.macs.contains(&parsed.mac) {
                                        entry.macs.push(parsed.mac.clone());
                                    }
                                    if let Some(ref g) = parsed.gnw_info {
                                        let lat = g.latitude  as f64 * LAT_LON_SCALE;
                                        let lon = g.longitude as f64 * LAT_LON_SCALE;
                                        // reject sentinel GPS fixes (no lock = 0,0 or out-of-range)
                                        if lat.abs() <= 89.9 && lon.abs() <= 179.9 && !(lat == 0.0 && lon == 0.0) {
                                            entry.lat = Some(lat);
                                            entry.lon = Some(lon);
                                        }
                                        entry.speed_kmh = cam.and_then(|c| c.speed_kmh).or_else(|| {
                                            if g.speed >= 0x7FFF { None } else { Some(g.speed as f64 * SPEED_SCALE) }
                                        });
                                        entry.heading_deg = if g.heading >= 3601 { None } else { Some(g.heading as f64 * HEADING_SCALE) };
                                    }
                                    entry.is_ego = is_ego;
                                    entry.last_seen_ms = parsed.timestamp_ms;
                                }

                                // check tracking-warning thresholds for foreign vehicles
                                if !is_ego {
                                    if let Some(vid) = tracker.get_vid_for_mac(&parsed.mac) {
                                        let lat = parsed.gnw_info.as_ref()
                                            .map(|g| g.latitude  as f64 * LAT_LON_SCALE);
                                        let lon = parsed.gnw_info.as_ref()
                                            .map(|g| g.longitude as f64 * LAT_LON_SCALE);
                                        if let Some(reason) = checker.check_packet(
                                            vid, parsed.timestamp_ms, lat, lon,
                                        ) {
                                            let msg = match reason {
                                                WarnReason::Duration => tts::TtsMessage::TrackingDurationWarning,
                                                WarnReason::Distance => tts::TtsMessage::TrackingDistanceWarning,
                                            };
                                            self.announce(msg);
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        storage.mark_incomplete();
                        current_stats.is_incomplete = true;
                        current_stats.dropped_fragments += 1;
                    }
                }
                ReassemblyStatus::Dropped => {
                    storage.mark_incomplete();
                    current_stats.is_incomplete = true;
                    current_stats.dropped_fragments += 1;
                }
                ReassemblyStatus::Pending => {}
            }

            // throttled stats write: skip if last write was less than 100 ms ago
            if stats_ts.elapsed() >= STATS_INTERVAL {
                *self.pcap_stats.write() = current_stats;
                stats_ts = Instant::now();
            }

            // periodic timeout check for lost foreign vehicles
            if timeout_ts.elapsed() >= TIMEOUT_CHECK_INTERVAL {
                checker.update_config(self.tracking_warning_cfg.read().clone());
                let timeout_ms = *self.foreign_vehicle_timeout_ms.read();
                let lost = tracker.drain_lost_vehicles(timeout_ms);
                if !lost.is_empty() {
                    for &vid in &lost {
                        checker.remove_vehicle(vid);
                    }
                    for _ in 0..lost.len() {
                        self.announce(tts::TtsMessage::ForeignVehicleLost);
                    }
                }
                timeout_ts = Instant::now();
            }

            // batch-write accumulated timeline points every 500 ms
            if tl_ts.elapsed() >= TIMELINE_WRITE_INTERVAL && !pending.is_empty() {
                let mut tl = self.mac_timeline.write();
                tl.extend(pending.drain(..));
                // trim oldest entries when over the cap
                if tl.len() > TIMELINE_MAX_POINTS {
                    let excess = tl.len() - TIMELINE_MAX_POINTS;
                    tl.drain(..excess);
                }
                tl_ts = Instant::now();
            }

            // throttled live-vehicle signal write (every 500 ms)
            if live_ts.elapsed() >= LIVE_WRITE_INTERVAL {
                let snapshot: Vec<LiveVehicleState> = veh_map.values().cloned().collect();
                *self.live_vehicles.write() = snapshot;
                live_ts = Instant::now();
            }
        }

        // final flush and session cleanup
        *self.pcap_stats.write() = current_stats;
        *self.live_vehicles.write() = Vec::new();
        self.current_capture_path.set(None);

        let _ = storage.cleanup();
        let _ = storage.close();
    }
}

// sse stream adapter

/// Converts an SSE HTTP response into the fragment stream expected by
/// `start_pcap_reassembly_loop`.
///
/// Each SSE `data:` line must contain a hex-encoded packed PCAP message.
/// The message is wrapped as a single-fragment BLE packet so it passes through `BleReassembler` without reassembly overhead.
fn sse_to_fragment_stream(resp: reqwest::Response) -> impl futures::Stream<Item = Vec<u8>> + Unpin {
    let stream = Box::pin(
        resp.bytes_stream()
            .filter_map(|r| futures::future::ready(r.ok())),
    );
    Box::pin(futures::stream::unfold(
        (stream, String::new()),
        |(mut stream, mut buf)| async move {
            loop {
                match parse_sse_fragment(&mut buf) {
                    Some(Some(frag)) => return Some((frag, (stream, buf))),
                    Some(None) => continue, // non-data SSE event; consume and loop
                    None => match stream.next().await {
                        Some(chunk) => {
                            if let Ok(s) = std::str::from_utf8(&chunk) {
                                buf.push_str(s);
                            }
                        }
                        None => return None,
                    },
                }
            }
        },
    ))
}

/// Attempts to extract one SSE fragment from `buf`.
///
/// Returns `None` when no complete event (`\n\n` boundary) is present.
/// Returns `Some(None)` when an event was consumed but contained no `data:` line.
/// Returns `Some(Some(frag))` when a fragment was successfully parsed.
///
/// Always drains the consumed event from `buf` on `Some(_)` so the caller can immediately loop without reparsing already-seen bytes.
fn parse_sse_fragment(buf: &mut String) -> Option<Option<Vec<u8>>> {
    let event_end = buf.find("\n\n")?;
    let frag = buf[..event_end]
        .lines()
        .find(|l| l.starts_with("data: "))
        .and_then(|l| core_logic::ble_protocol::hex_decode(&l["data: ".len()..]))
        .and_then(|packed| core_logic::ble_protocol::to_single_fragment(&packed));
    buf.drain(..event_end + 2);
    Some(frag)
}
