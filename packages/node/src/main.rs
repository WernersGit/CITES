pub mod config;
pub mod capture;
pub mod replay;
pub mod injection;
mod logger;

use api::metrics::{MetricsService, NodeStatus, SystemMetrics, TrackingReport, VirtualVehicle};
use capture::CaptureDispatcher;
use replay::ReplayEngine;
use api::ble_constants::*;
use config::AppMode;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error};
use logger::NodeLogger;
use core_logic::config::NodeConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_guard = NodeLogger::init();

    // apply persisted runtime overrides (log level, port) from previous push
    let rt = config::RuntimeConfig::load();
    if let Some(ref level) = rt.log_level {
        if let Err(e) = NodeLogger::set_level(level) {
            eprintln!("[NodeLogger] couldn't restore log level: {e}");
        }
    }

    let mut cfg = config::AppConfig::load().expect("Failed to load configuration. Ensure config.toml exists or ENV vars are set."); // intentional — nothing works without a config

    // apply port override from runtime config
    if let Some(port) = rt.api_port {
        info!("API port overriden to {port} (from runtime config)");
        cfg.interfaces.network_port = port;
    }

    info!("Strating node '{}' in {:?} mode", cfg.name, cfg.mode);

    match cfg.mode {
        AppMode::Cloud => run_cloud(cfg).await,
        AppMode::Local => run_local(cfg).await,
    }
}

//cloud node
//
// a Cloud node cannot capture directly. it reads already-stored `.pcapng`
// files from `./captures` (same layout as a Local node) and
// produces tracking reports on demand via GET /tracking/report

async fn run_cloud(cfg: config::AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    info!("Cloud mode: capture device disabled.  Serving tracking reports from ./captures");

    if !cfg.interfaces.enable_network_api {
        error!("Cloud mode requires enable_network_api = true in config.toml");
        return Ok(());
    }

    let port = cfg.interfaces.network_port;
    let metrics_svc = Arc::new(Mutex::new(MetricsService::new()));

    // cached NodeStatus (metrics only; replay_count is always 0 on Cloud)
    let cache: Arc<Mutex<NodeStatus>> = Arc::new(Mutex::new(NodeStatus {
        cpu_usage: 0.0,
        ram_used_mb: 0.0,
        ram_total_mb: 0.0,
        temp_celsius: 0.0,
        replay_count: 0,
    }));
    {
        let status_cache = Arc::clone(&cache);
        let svc = Arc::clone(&metrics_svc);
        tokio::spawn(async move {
            loop {
                let m = svc.lock().unwrap().gather_metrics();
                *status_cache.lock().unwrap() = NodeStatus {
                    cpu_usage: m.cpu_usage,
                    ram_used_mb: m.ram_used_mb,
                    ram_total_mb: m.ram_total_mb,
                    temp_celsius: m.temp_celsius,
                    replay_count: 0,
                };
                sleep(Duration::from_millis(500)).await;
            }
        });
    }

    use axum::{routing::{get, post}, Router, response::IntoResponse, extract::{Json, DefaultBodyLimit}, http::StatusCode};
    use tower_http::cors::CorsLayer;

    let app = Router::new()
        .route("/status", get({
            let s = Arc::clone(&cache);
            move || async move { Json(s.lock().unwrap().clone()).into_response() }
        }))
        // on-demand tracking report, runs on blocking thread pool
        .route("/tracking/report", get(tracking_report_handler))
        .route("/node/config", get(get_node_config_handler).post(node_config_handler))
        .route("/archive/latest", get(archive_latest))
        .route("/archive/list", get(archive_list))
        .route("/archive/download/{filename}", get(archive_download))
        .merge(
            Router::new()
                .route("/archive/upload/{filename}", post(archive_upload))
                .layer(DefaultBodyLimit::disable()),
        )
        .layer(CorsLayer::permissive());

    let addr = format!("0.0.0.0:{port}");
    if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
        info!("Cloud HTTP API listening on {addr}");
        axum::serve(listener, app).await?;
    }

    Ok(())
}

/// reads all pcapng archives from `./captures`, runs tracking, returns a report
async fn tracking_report_handler() -> impl axum::response::IntoResponse {
    use axum::{http::StatusCode, response::IntoResponse, Json};

    match tokio::task::spawn_blocking(build_tracking_report).await {
        Ok(report) => Json(report).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn build_tracking_report() -> TrackingReport {
    use core_logic::{ego_mac::EgoMac, vehicle_tracker::{VehicleTracker, PacketInfo, InsertResult, LAT_LON_SCALE, SPEED_SCALE, HEADING_SCALE}};
    use core_logic::pcap_parser::PcapParser;

    let dir = std::path::Path::new("./captures");

    //grab all archives in order
    let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("archive_"))
            .map(|e| e.path())
            .collect(),
        Err(_) => return empty_report(),
    };
    files.sort();

    let mut ego = EgoMac::new(10_000, 5);
    let mut tracker = VehicleTracker::new();
    let mut count: u64 = 0;

    for path in &files {
        let packets = match PcapParser::parse_file(path) {
            Ok(pkts) => pkts,
            Err(_) => continue,
        };

        for pkt in packets {
            ego.insert_measurement(pkt.timestamp_ms, pkt.mac.clone(), pkt.rssi);

            let info = PacketInfo {
                mac: pkt.mac.clone(),
                timestamp_ms: pkt.timestamp_ms,
                lat: pkt.gnw_info.as_ref().map(|g| g.latitude as f64 * LAT_LON_SCALE),
                lon: pkt.gnw_info.as_ref().map(|g| g.longitude as f64 * LAT_LON_SCALE),
                pos_confidence_m: None,
                speed_kmh: pkt.gnw_info.as_ref().map(|g| g.speed as f64 * SPEED_SCALE),
                spd_conf:          None,
                heading_deg: pkt.gnw_info.as_ref().map(|g| g.heading as f64 * HEADING_SCALE),
                hdg_conf:          None,
                yaw_rate:          None,
                yaw_conf:          None,
                accel:             None,
                brake:     None,
                gas:       None,
                curvature:  None,
                v_len:      None,
                v_wid:      None,
                frame_seq:  pkt.frame_seq,
            };
            tracker.insert_packet(info);
            count += 1;
        }

        // refresh ego MACs after each file

        tracker.set_ego_macs(
            ego.evaluate().iter().map(|m| m.mac.clone())
        );
    }

    // for offline analysis we skip timeout-based eviction; all seen vehicles are reported
    let top = ego.evaluate();
    let ego_mac = top.first().map(|m| m.mac.clone());
    let stability = top.first().map(|m| m.stability_score).unwrap_or(0.0);

    let vehicles = tracker
        .iter_vehicles()
        .map(|(vid, macs)| VirtualVehicle { virtual_id: vid, macs: macs.to_vec() })
        .collect();

    TrackingReport {
        ego_mac,
        ego_stability_score: stability,
        vehicles,
        packets_analysed: count,
    }
}

fn empty_report() -> TrackingReport {
    TrackingReport {
        ego_mac: None,
        ego_stability_score: 0.0,
        vehicles: vec![],
        packets_analysed: 0,
    }
}
/// Handles `GET /node/config` — returns the current [`NodeConfig`] from `RuntimeConfig`.
async fn get_node_config_handler() -> impl axum::response::IntoResponse {
    use axum::Json;
    let runtime = config::RuntimeConfig::load();
    let node_config = NodeConfig {
        log_level: runtime.log_level
            .as_deref()
            .and_then(core_logic::config::LogLevel::from_filter_str)
            .unwrap_or_default(),
        api_port: runtime.api_port.unwrap_or(8080),
    };
    Json(node_config)
}

/// Handles `POST /node/config` — applies log level immediately and persists both
/// log level and port to [`config::RuntimeConfig`] for the next restart.
async fn node_config_handler(
    axum::extract::Json(node_cfg): axum::extract::Json<NodeConfig>,
) -> axum::http::StatusCode {
    // log level can be hot-swaped
    if let Err(e) = NodeLogger::set_level(node_cfg.log_level.as_filter_str()) {
        error!("log level change failed: {e}");
    } else {
        info!("Log level set to {}", node_cfg.log_level.as_filter_str());
    }

    // persist so the settings survive a restart
    let runtime = config::RuntimeConfig {
        log_level: Some(node_cfg.log_level.as_filter_str().to_string()),
        api_port:  Some(node_cfg.api_port),
    };
    if let Err(e) = runtime.save() {
        error!("couldn't save runtime config: {e}");
    }

    if node_cfg.api_port != 8080 {
        warn!(
            "API port changed to {} — restart the node to apply.",
            node_cfg.api_port
        );
    }

    axum::http::StatusCode::OK
}

// local node
//
// captures live packets, archives them, streams via BLE/HTTP, and supports
// replay. tracking is done client-side

async fn run_local(cfg: config::AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting Capture Dispatcher on interface: {}", cfg.interfaces.capture_interface);
    let dispatcher = CaptureDispatcher::new(
        cfg.interfaces.capture_interface.clone(),
    );
    // clone the filter Arc before `start()` consumes the dispatcher
    let inj_filter = dispatcher.injection_filter.clone();
    let channels = dispatcher.start();

    // broadcast fan-out
    // blocking task bridges the crossbeam channel to a tokio broadcast
    // so BLE and HTTP SSE subscribers both receive every packet independently.
    // the anchor rx prevents early channel closure before anyone has subscribed
    let (pcap_broadcast_tx, _pcap_broadcast_rx_anchor) =
        tokio::sync::broadcast::channel::<crate::capture::PcapMessage>(64);
    {
        let fan_tx = pcap_broadcast_tx.clone();
        let live_rx = channels.live_rx;
        tokio::task::spawn_blocking(move || {
            while let Ok(msg) = live_rx.recv() {
                let _ = fan_tx.send(msg);
            }
        });
    }

    let replay_engine = ReplayEngine::new();
    let replay_cfg = replay_engine.config_handle();
    let replay_cnt = replay_engine.replay_count_handle();
    // replay sends on the capture interface; injection uses its own iface below
    replay_engine.start(
        channels.replay_rx,
        cfg.interfaces.capture_interface.clone(),
        inj_filter.clone(),
    );
    info!("ReplayEngnie started.");

    // cached NodeStatus
    let cache: Arc<Mutex<NodeStatus>> = Arc::new(Mutex::new(NodeStatus {
        cpu_usage: 0.0,
        ram_used_mb: 0.0,
        ram_total_mb: 0.0,
        temp_celsius: 0.0,
        replay_count: 0,
    }));

    {
        let status_cache = Arc::clone(&cache);
        let cnt = Arc::clone(&replay_cnt);
        let metrics_svc = Arc::new(Mutex::new(MetricsService::new()));
        tokio::spawn(async move {
            loop {
                let m = metrics_svc.lock().unwrap().gather_metrics();
                let r = cnt.load(Ordering::Relaxed);
                *status_cache.lock().unwrap() = NodeStatus {
                    cpu_usage: m.cpu_usage,
                    ram_used_mb: m.ram_used_mb,
                    ram_total_mb: m.ram_total_mb,
                    temp_celsius: m.temp_celsius,
                    replay_count: r,
                };
                sleep(Duration::from_millis(500)).await;
            }
        });
    }

    // shared injection state: one active run at a time, accessible from BLE and HTTP
    let active_injection: Arc<Mutex<Option<injection::ActiveInjection>>> =
        Arc::new(Mutex::new(None));
    let inj_iface = cfg.interfaces.injection_interface.clone()
        .unwrap_or_else(|| cfg.interfaces.capture_interface.clone());
    info!("Injection interface: {inj_iface}");

    if cfg.interfaces.enable_bluetooth {
        #[cfg(target_os = "linux")]
        {
            info!("Initializing BLE GATT Server...");
            let pcap_tx      = pcap_broadcast_tx.clone();
            let status       = Arc::clone(&cache);
            let replay       = Arc::clone(&replay_cfg);
            let active       = Arc::clone(&active_injection);
            let iface        = inj_iface.clone();
            let ble_filter   = inj_filter.clone();
            tokio::spawn(async move {
                if let Err(e) = linux_ble::start_server(pcap_tx, status, replay, active, iface, ble_filter).await {
                    error!("BLE server exited with error: {e}");
                }
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            error!("BLE Server is unsupported on this OS. Skipping BLE initialization.");
        }
    } else {
        info!("Bluetooth interface disabled in config.toml.");
    }

    if cfg.interfaces.enable_network_api {
        info!("Initializing Network API on port {}...", cfg.interfaces.network_port);

        let port            = cfg.interfaces.network_port;
        let net_status      = Arc::clone(&cache);
        let net_replay_cfg  = Arc::clone(&replay_cfg);
        let net_injection   = Arc::clone(&active_injection);
        let net_iface       = inj_iface.clone();
        let net_pcap_tx     = pcap_broadcast_tx.clone();
        let net_inj_filter  = inj_filter.clone();

        tokio::spawn(async move {
            let active_inj = net_injection;
            let inj_iface = net_iface;
            let inj_filter = net_inj_filter;
            use axum::{
                routing::{get, post},
                Router,
                response::IntoResponse,
                extract::{Json, DefaultBodyLimit, State},
                http::StatusCode,
            };

            use tower_http::cors::CorsLayer;
            use core_logic::config::{InjectionConfig, InjectionStatus, RepeatModeConfig};

            // wrap the broadcast sender in arc for use as Axum State
            let pcap_tx_state = Arc::new(net_pcap_tx);

            let app = Router::new()
                .route("/status", get({
                    let s = Arc::clone(&net_status);
                    move || async move { Json(s.lock().unwrap().clone()).into_response() }
                }))
                .route("/metrics", get({
                    let s = Arc::clone(&net_status);
                    move || async move {
                        let snap = s.lock().unwrap().clone();
                        format!("{:.1},{:.1},{:.1},{:.1},FF:FF:FF:FF:FF:FF",
                            snap.cpu_usage, snap.ram_used_mb,
                            snap.ram_total_mb, snap.temp_celsius)
                            .into_response()
                    }
                }))
                .route("/replay/count", get({
                    let s = Arc::clone(&net_status);
                    move || async move { s.lock().unwrap().replay_count.to_string() }
                }))
                .route("/replay/config", post({
                    let cfg_h = Arc::clone(&net_replay_cfg);
                    move |Json(new_cfg): Json<RepeatModeConfig>| async move {
                        if let Ok(mut guard) = cfg_h.lock() { *guard = new_cfg; }
                        StatusCode::OK
                    }
                }))
                // live PCAP SSE stream
                .route("/pcap/stream", get(pcap_sse_handler))
                // injection endpoints
                .route("/injection/start", post({
                    let active = Arc::clone(&active_inj);
                    let iface  = inj_iface.clone();
                    let filter = inj_filter.clone();
                    move |Json(cfg): Json<InjectionConfig>| async move {
                        // stop any previous run first
                        if let Some(old) = active.lock().unwrap().take() {
                            old.stop();
                        }
                        let handle = injection::start_injection(cfg, iface.clone(), Some(filter.clone()));
                        *active.lock().unwrap() = Some(handle);
                        StatusCode::OK
                    }
                }))
                .route("/injection/stop", post({
                    let active = Arc::clone(&active_inj);
                    move || async move {
                        if let Some(a) = active.lock().unwrap().as_ref() {
                            a.stop();
                        }
                        StatusCode::OK
                    }
                }))
                .route("/injection/pause", post({
                    let active = Arc::clone(&active_inj);
                    move || async move {
                        if let Some(a) = active.lock().unwrap().as_ref() {
                            a.toggle_pause();
                        }
                        StatusCode::OK
                    }
                }))
                .route("/injection/filter", post({
                    let active = Arc::clone(&active_inj);
                    move |Json(on): Json<bool>| async move {
                        if let Some(a) = active.lock().unwrap().as_ref() {
                            a.set_filter(on);
                        }
                        StatusCode::OK
                    }
                }))
                .route("/injection/status", get({
                    let active = Arc::clone(&active_inj);
                    move || async move {
                        let status = active
                            .lock().unwrap()
                            .as_ref()
                            .map(|a| a.status.lock().unwrap().clone())
                            .unwrap_or_default();
                        Json(status).into_response() 
                    }
                }))
                .route("/node/config", get(get_node_config_handler).post(node_config_handler))
                .route("/archive/latest", get(archive_latest))
                .route("/archive/list", get(archive_list))
                .route("/archive/download/{filename}", get(archive_download))
                .merge(
                    Router::new()
                        .route("/archive/upload/{filename}", post(archive_upload))
                        .layer(DefaultBodyLimit::disable()),
                )
                .with_state(pcap_tx_state)
                .layer(CorsLayer::permissive());

            let addr = format!("0.0.0.0:{port}");
            if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
                info!("HTTP API listening on {}", addr);
                let _ = axum::serve(listener, app).await;
            }
        });
    }

    #[cfg(target_os = "linux")]
    {
        // BLE and HTTP API run as spawned tasks -> park the main task to keep the runtime alive
        std::future::pending::<()>().await;
    }

    #[cfg(not(target_os = "linux"))]
    {
        info!("Starting mock metrics loop (non-Linux)...");
        loop {
            let snap = cache.lock().unwrap().clone();
            info!("CPU: {:.2}%, RAM: {:.2}/{:.2} MB, Temp: {:.2} C, Replayed: {}",
                snap.cpu_usage, snap.ram_used_mb, snap.ram_total_mb,
                snap.temp_celsius, snap.replay_count);
            sleep(Duration::from_secs(2)).await;
        }
    }

    Ok(())
}

/// Streams live PCAP packets as Server-Sent Events
///
/// Each event carries one hex-encoded packed message (`pack_pcap_message` format)
async fn pcap_sse_handler(
    axum::extract::State(tx): axum::extract::State<Arc<tokio::sync::broadcast::Sender<crate::capture::PcapMessage>>>,
) -> impl axum::response::IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use core_logic::ble_protocol::{hex_encode, pack_pcap_message};
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt as TokioStreamExt;

    let rx = tx.subscribe();
    let sse_stream = BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(msg) => {
                let packed = pack_pcap_message(msg.sequence_number, msg.timestamp_ns, &msg.data);
                Some(Ok::<Event, std::convert::Infallible>(Event::default().data(hex_encode(&packed))))
            }
            Err(_) => None,
        }
    });

    Sse::new(sse_stream).keep_alive(KeepAlive::default())
}

/// Streams the most recent archive PCAPNG file as a binary download
async fn archive_latest() -> impl axum::response::IntoResponse {
    use axum::{http::StatusCode, response::IntoResponse};

    let dir = std::path::Path::new("./captures");
    let latest = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("archive_"))
            .max_by_key(|e| e.file_name()),
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let entry = match latest {
        Some(e) => e,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let filename = entry.file_name().to_string_lossy().into_owned();

    match tokio::fs::read(entry.path()).await {
        Ok(data) => axum::response::Response::builder()
            .header("Content-Type", "application/octet-stream")
            .header("Content-Disposition", format!("attachment; filename=\"{}\"", filename))
            .body(axum::body::Body::from(data))
            .unwrap()
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Returns a JSON array of archive filenames available in `./captures`
async fn archive_list() -> impl axum::response::IntoResponse {
    use axum::{response::IntoResponse, Json};

    let dir = std::path::Path::new("./captures");
    let mut files: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("archive_"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort_by(|a, b| b.cmp(a));
    Json(files).into_response()
}

/// Downloads a specific archive file by filename.  Rejects path traversal attempts
async fn archive_download(
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    use axum::{http::StatusCode, response::IntoResponse};

    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = std::path::Path::new("./captures").join(&filename);
    match tokio::fs::read(&path).await {
        Ok(data) => axum::response::Response::builder()
            .header("Content-Type", "application/octet-stream")
            .header("Content-Disposition", format!("attachment; filename=\"{}\"", filename))
            .body(axum::body::Body::from(data))
            .unwrap()
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Accepts a raw PCAPNG file upload and stores it in `./captures`
async fn archive_upload(
    axum::extract::Path(filename): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    use axum::{http::StatusCode, response::IntoResponse};

    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let dir = std::path::Path::new("./captures");
    if !dir.exists() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    let path = dir.join(&filename);
    match tokio::fs::write(&path, &body).await {
        Ok(_)  => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// linux BLE GATT Server

#[cfg(target_os = "linux")]
mod linux_ble {
    use super::*;
    use bluer::{
        adv::Advertisement,
        gatt::{
            local::{
                Application, Characteristic, CharacteristicNotify, CharacteristicNotifyMethod,
                CharacteristicNotifier, CharacteristicRead, CharacteristicWrite,
                CharacteristicWriteMethod, ReqError, Service,
            },
        },
    };
    use tokio::sync::broadcast;
    use crate::capture::{InjectionFrameFilter, PcapMessage};
    use core_logic::ble_protocol::{pack_pcap_message, fragment_payload, BLE_MAX_CHUNK_SIZE};
    use core_logic::ble_handshake::{
        self, HandshakeFrame, CLIENT_FALLBACK_CHUNK, PROTOCOL_VERSION,
    };
    use core_logic::config::{InjectionConfig, RepeatModeConfig};
    use crate::replay::ReplayConfigHandle;
    use crate::injection::ActiveInjection;
    use api::ble_constants::{
        ARCHIVE_LIST_CHARACTERISTIC_UUID, HANDSHAKE_CHARACTERISTIC_UUID,
        INJECTION_STATUS_CHARACTERISTIC_UUID, NODE_CONFIG_CHARACTERISTIC_UUID,
        BLE_CMD_MAGIC, BLE_CMD_START_INJECTION, BLE_CMD_STOP_INJECTION,
        BLE_CMD_PAUSE_INJECTION, BLE_CMD_SET_INJ_FILTER, BLE_CMD_SET_NODE_CONFIG,
    };
    use std::sync::atomic::{AtomicU16, Ordering};
    use uuid::Uuid;

    pub async fn start_server(
        pcap_tx: broadcast::Sender<PcapMessage>,
        cached_status: Arc<Mutex<NodeStatus>>,
        replay_cfg: ReplayConfigHandle,
        active_injection: Arc<Mutex<Option<ActiveInjection>>>,
        inj_iface: String,
        inj_filter: InjectionFrameFilter,
    ) -> bluer::Result<()> {
        let session = bluer::Session::new().await?;
        let adapter = session.default_adapter().await?;
        adapter.set_powered(true).await?;

        info!("Bluetooth adapter {} powered on.", adapter.name());

        let cites_service    = Uuid::parse_str(CITES_SERVICE_UUID).unwrap();
        let metrics_char     = Uuid::parse_str(METRICS_CHARACTERISTIC_UUID).unwrap();
        let pcap_char        = Uuid::parse_str(PCAP_CHARACTERISTIC_UUID).unwrap();
        let command_char     = Uuid::parse_str(COMMAND_CHARACTERISTIC_UUID).unwrap();
        let archive_list_char = Uuid::parse_str(ARCHIVE_LIST_CHARACTERISTIC_UUID).unwrap();
        let inj_status_char  = Uuid::parse_str(INJECTION_STATUS_CHARACTERISTIC_UUID).unwrap();
        let node_config_char = Uuid::parse_str(NODE_CONFIG_CHARACTERISTIC_UUID).unwrap();
        let handshake_char   = Uuid::parse_str(HANDSHAKE_CHARACTERISTIC_UUID).unwrap();

        // Session state shared by handshake handlers and the PCAP notifier.
        // Initial value = CLIENT_FALLBACK_CHUNK (244 B, BLE 5.0 DLE default)
        // so an un-handshaken client receives traffic that fits any BLE 5.0 link.
        // The handshake WRITE replaces this with the probe-confirmed value.
        let max_chunk = BLE_MAX_CHUNK_SIZE;
        let negotiated_chunk = Arc::new(AtomicU16::new(
            CLIENT_FALLBACK_CHUNK.min(max_chunk),
        ));

        let adv = Advertisement {
            advertisement_type: bluer::adv::Type::Peripheral,
            service_uuids: vec![cites_service].into_iter().collect(),
            discoverable: Some(true),
            local_name: Some(format!("{}Pi", CITES_MAC_NAME_PREFIX)),
            ..Default::default() 
        };
        let _adv_handle = adapter.advertise(adv).await?;
        info!("BLE advertising started.");

        let app = Application {
            services: vec![Service {
                uuid: cites_service,
                primary: true,
                characteristics: vec![
                    // metrics (READ)
                    Characteristic {
                        uuid: metrics_char,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(move |_req| {
                                let cache = Arc::clone(&cached_status);
                                Box::pin(async move {
                                    let snap = cache.lock().unwrap().clone();
                                    Ok(snap.to_csv().into_bytes())
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    // live PCAP stream
                    Characteristic {
                        uuid: pcap_char,
                        notify: Some(CharacteristicNotify {
                            notify: true,
                            method: CharacteristicNotifyMethod::Fun(Box::new({
                                let pcap_tx = pcap_tx.clone();
                                let chunk_atomic = Arc::clone(&negotiated_chunk);
                                move |mut notifier: CharacteristicNotifier| {
                                    let tx = pcap_tx.clone();
                                    let chunk_atomic = Arc::clone(&chunk_atomic);
                                    Box::pin(async move {
                                        let mut rx = tx.subscribe();
                                        info!("BLE notify: PCAP subscription active, starting notify loop");
                                        // count consecutive notify failures; exit only after 5 in a row so transient BlueZ queue-full errors or brief rf drops don't permanently kill the notification stream
                                        let mut consec_fail: u32 = 0;
                                        'notify: loop {
                                            match rx.recv().await {
                                                Ok(msg) => {
                                                    // re-read every packet so a mid-session handshake update takes effect immediately
                                                    let chunk_size =
                                                        chunk_atomic.load(Ordering::Relaxed) as usize;
                                                    let packed = pack_pcap_message(
                                                        msg.sequence_number,
                                                        msg.timestamp_ns,
                                                        &msg.data,
                                                    );
                                                    let frags = fragment_payload(
                                                        msg.sequence_number,
                                                        &packed,
                                                        chunk_size,
                                                    );
                                                    let mut frame_ok = true;
                                                    for frag in frags {
                                                        if notifier.notify(frag).await.is_err() {
                                                            frame_ok = false;
                                                            break;
                                                        }
                                                    }
                                                    if frame_ok {
                                                        if consec_fail > 0 {
                                                            info!("BLE notify: recovered after {} consecutive failure(s)", consec_fail);
                                                            consec_fail = 0;
                                                        }
                                                    } else {
                                                        consec_fail += 1;
                                                        warn!("BLE notify: failed for seq {} (consec={})", msg.sequence_number, consec_fail);
                                                        if consec_fail >= 5 {
                                                            warn!("BLE notify: 5 consecutive failures — subscriber disconnected, exiting loop");
                                                            break 'notify;
                                                        }
                                                    }
                                                }
                                                Err(broadcast::error::RecvError::Lagged(_)) => {
                                                    continue;
                                                }
                                                Err(_) => break,
                                            }
                                        }
                                        info!("BLE notify: PCAP notify loop exited");
                                    })
                                }
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    // command (WRITE)
                    Characteristic {
                        uuid: command_char,
                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Fun({
                                let active_injection = Arc::clone(&active_injection);
                                let inj_filter = inj_filter.clone();
                                Box::new(move |new_val: Vec<u8>, _req| {
                                let cfg_h  = Arc::clone(&replay_cfg);
                                let active = Arc::clone(&active_injection);
                                let iface  = inj_iface.clone();
                                let filter = inj_filter.clone();
                                Box::pin(async move {
                                    // extended injection commands take priority:
                                    // check magic byte first so 0xFF is never misinterpreted as a RepeatModeConfig flags byte
                                    match (new_val.first().copied(), new_val.get(1).copied()) {
                                        (Some(BLE_CMD_MAGIC), Some(BLE_CMD_SET_NODE_CONFIG)) => {
                                            if let Some(node_cfg) = NodeConfig::from_ble_binary(&new_val) {
                                                if let Err(e) = NodeLogger::set_level(node_cfg.log_level.as_filter_str()) {
                                                    error!("BLE: failed to apply log level: {e}");
                                                } else {
                                                    info!("BLE: log level set to {}", node_cfg.log_level.as_filter_str());
                                                }
                                                let runtime = crate::config::RuntimeConfig {
                                                    log_level: Some(node_cfg.log_level.as_filter_str().to_string()),
                                                    api_port:  Some(node_cfg.api_port),
                                                };
                                                if let Err(e) = runtime.save() {
                                                    error!("BLE: failed to save runtime config: {e}");
                                                }
                                                if node_cfg.api_port != 8080 {
                                                    warn!("BLE: API port changed to {} — restart required.", node_cfg.api_port);
                                                }
                                            } else {
                                                return Err(ReqError::InvalidValueLength);
                                            }
                                        }
                                        (Some(BLE_CMD_MAGIC), Some(BLE_CMD_START_INJECTION)) => {
                                            if let Some(cfg) = InjectionConfig::from_ble_binary(&new_val) {
                                                let mut guard = active.lock().unwrap();
                                                if let Some(old) = guard.take() { old.stop(); }
                                                *guard = Some(crate::injection::start_injection(cfg, iface, Some(filter)));
                                                info!("Injection started via BLE" );
                                            } else {
                                                return Err(ReqError::InvalidValueLength);
                                            }
                                        }
                                        (Some(BLE_CMD_MAGIC), Some(BLE_CMD_STOP_INJECTION)) => {
                                            if let Some(a) = active.lock().unwrap().as_ref() {
                                                a.stop();
                                            }
                                        }
                                        (Some(BLE_CMD_MAGIC), Some(BLE_CMD_PAUSE_INJECTION)) => {
                                            if let Some(a) = active.lock().unwrap().as_ref() {
                                                a.toggle_pause();
                                            }
                                        }
                                        (Some(BLE_CMD_MAGIC), Some(BLE_CMD_SET_INJ_FILTER)) => {
                                            let on = new_val.get(2).copied().unwrap_or(1) != 0;
                                            if let Some(a) = active.lock().unwrap().as_ref() {
                                                a.set_filter(on);
                                            }
                                        }
                                        // legacy RepeatModeConfig (first byte is always <= 0x03)
                                        _ => {
                                            if let Some(cfg) = RepeatModeConfig::from_ble_binary(&new_val) {
                                                if let Ok(mut guard) = cfg_h.lock() {
                                                    *guard = cfg;
                                                    info!("ReplayConfig updated via BLE");
                                                }
                                            } else {
                                                return Err(ReqError::NotSupported);
                                            }
                                        }
                                    }
                                    Ok(())
                                })
                            })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },

                    // archive list with packet counts (READ)
                    Characteristic {
                        uuid: archive_list_char,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(move |_req| {
                                Box::pin(async move {
                                    Ok(build_archive_list_payload().into_bytes())
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },

                    // injection status (READ)
                    Characteristic {
                        uuid: inj_status_char,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(move |_req| {
                                let active = Arc::clone(&active_injection);
                                Box::pin(async move {
                                    let status = active.lock().unwrap()
                                        .as_ref()
                                        .map(|a| a.status.lock().unwrap().clone())
                                        .unwrap_or_default();
                                    Ok(status.to_ble_binary().to_vec())
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    // node config (READ)
                    Characteristic {
                        uuid: node_config_char,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(move |_req| {
                                Box::pin(async move {
                                    let runtime = crate::config::RuntimeConfig::load();
                                    let node_config = NodeConfig {
                                        log_level: runtime.log_level
                                            .as_deref()
                                            .and_then(core_logic::config::LogLevel::from_filter_str)
                                            .unwrap_or_default(),
                                        api_port: runtime.api_port.unwrap_or(8080),
                                    };
                                    Ok(node_config.to_ble_status().to_vec())
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },

                    // session handshake (READ + WRITE)
                    // WRITE: client request -> reconciled and stored as the active session chunk size
                    // READ: returns the current session frame (server view)
                    Characteristic {
                        uuid: handshake_char,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new({
                                let chunk_atomic = Arc::clone(&negotiated_chunk);
                                move |_req| {
                                    let chunk_atomic = Arc::clone(&chunk_atomic);
                                    Box::pin(async move {
                                        let frame = HandshakeFrame {
                                            version: PROTOCOL_VERSION,
                                            max_chunk: chunk_atomic.load(Ordering::Relaxed),
                                            capabilities: 0,
                                        };
                                        Ok(frame.to_bytes().to_vec())
                                    })
                                }
                            }),
                            ..Default::default()
                        }),

                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Fun(Box::new({
                                let chunk_atomic = Arc::clone(&negotiated_chunk);
                                let server_max = max_chunk;
                                move |new_val: Vec<u8>, _req| {
                                    let chunk_atomic = Arc::clone(&chunk_atomic);
                                    Box::pin(async move {
                                        match HandshakeFrame::from_bytes(&new_val) {
                                            Some(req) => {
                                                let agreed = ble_handshake::reconcile(server_max, req);
                                                chunk_atomic.store(
                                                    agreed.max_chunk,
                                                    Ordering::Relaxed,
                                                );
                                                info!(
                                                    "BLE handshake: client v{} max={}B -> negotiated v{} max={}B (server cap {}B)",
                                                    req.version, req.max_chunk,
                                                    agreed.version, agreed.max_chunk,
                                                    server_max,
                                                );
                                                Ok(())
                                            }
                                            None => Err(ReqError::InvalidValueLength),
                                        }
                                    })
                                }
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let _app_handle = adapter.serve_gatt_application(app).await?;
        info!(
            "GATT server running (cap {}B, initial {}B/chunk; chunk size negotiated per session). Waiting for connections from the CITES Client.",
            max_chunk,
            negotiated_chunk.load(Ordering::Relaxed),
        );

         std::future::pending::<()>().await;
        Ok(())
    }

    /// scans `./captures`, parses each archive, and returns a formatted list
    ///
    /// each line: `filename:valid_packet_count\n`
    fn build_archive_list_payload() -> String {
        use core_logic::pcap_parser::PcapParser;

        let dir = std::path::Path::new("./captures");
        let mut entries: Vec<String> = match std::fs::read_dir(dir) {
            Ok(e) => e.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("archive_"))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => return String::new(),
        };
        entries.sort_by(|a, b| b.cmp(a));

        entries.iter().map(|name| {
            let path = dir.join(name);
            let count = std::fs::read(&path)
                .ok()
                .and_then(|b| PcapParser::parse_bytes_raw(&b).ok())
                .map(|pkts| pkts.len())
                .unwrap_or(0);
            format!("{name}:{count}\n")
        }).collect()
    }
}
