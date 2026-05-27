use dioxus::prelude::*;
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use core_logic::pcap_parser::ParsedPacket;
use core_logic::ego_mac::EgoMac;
use core_logic::vehicle_tracker::{VehicleTracker, PacketInfo, LAT_LON_SCALE, SPEED_SCALE, HEADING_SCALE};
use core_logic::parser::decoder::ItsPayload;
use platform::ConnectionService;
use platform::stats::LiveVehicleState;
use crate::source_picker::SourcePicker;
use crate::trajectory::{build_trajectories, build_playback_timeline, PlaybackPoint};
pub use crate::trajectory::DriveDirection;

pub mod car_view;
pub mod map_view;

pub use car_view::CarView;
pub use map_view::{MapView, LiveTrajData};

const LIVE_CSS: Asset = asset!("/assets/styling/live.css");

static DEMO_VEHICLES: &[&str] = &["VV-001", "VV-002", "VV-003", "VV-004"];

const TRAJ_COLORS: &[&str] = &[
    "#44aaff", "#ff7f0e", "#2ca02c", "#d62728",
    "#9467bd", "#8c564b", "#e377c2", "#17becf",
];

/// Interval between playback ticks in milliseconds.
const PLAYBACK_TICK_MS: u64 = 50;

/// Half-width of the visibility window used for the foreign-MAC list.
const VISIBLE_MAC_WINDOW_MS: i64 = 1_000;

/// Interval between online pcap re-parses in milliseconds.
const ONLINE_POLL_MS: u64 = 100;

/// Interval between online render ticks in milliseconds.
const ONLINE_TICK_MS: u64 = 50;

/// Initial render-clock lag behind the latest packet (approx. one CAM interval).
const ONLINE_LAG_INIT_MS: i64 = 100;

/// Snap forward if the render clock lags more than this behind the latest packet.
const ONLINE_LAG_MAX_MS: i64 = 500;

// mode

#[derive(Clone, PartialEq)]
enum LiveMode {
    Demo,
    Online,
    Offline,
}

impl LiveMode {
    fn label(&self) -> &'static str {
        match self {
            LiveMode::Demo    => "Demo",
            LiveMode::Online  => "Online",
            LiveMode::Offline => "Offline",
        }
    }

    fn next(&self) -> LiveMode {
        match self {
            LiveMode::Demo    => LiveMode::Online,
            LiveMode::Online  => LiveMode::Offline,
            LiveMode::Offline => LiveMode::Demo,
        }
    }
}

// vehicle state

/// Complete real-time state of a monitored vehicle: lights, ADAS, and telemetry.
///
/// In Demo/Online mode the telemetry fields (`mac`, `speed_kmh`, `heading_deg`, `drive_direction`) carry their zero/`None` defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleState {
    // exterior lights
    pub no_light: bool,
    pub daytime_running: bool,
    pub low_beam: bool,
    pub high_beam: bool,
    pub left_blinker: bool,
    pub right_blinker: bool,
    pub hazard: bool,
    // motion
    pub brake: bool,
    pub accelerating: bool,
    // adas
    pub acc_engaged: bool,
    pub cruise_control_active: bool,
    pub speed_limiter_active: bool,
    // telemetry (populated during playback)
    pub mac: String,
    pub speed_kmh: Option<f64>,
    pub heading_deg: Option<f64>,
    pub drive_direction: Option<DriveDirection>,
}

impl Default for VehicleState {
    fn default() -> Self {
        Self {
            no_light: false,
            daytime_running: true,
            low_beam: false,
            high_beam: false,
            left_blinker: false,
            right_blinker: false,
            hazard: false,
            brake: false,
            accelerating: false,
            acc_engaged: false,
            cruise_control_active: false,
            speed_limiter_active: false,
            mac: String::new(),
            speed_kmh: None,
            heading_deg: None,
            drive_direction: None,
        }
    }
}

impl VehicleState {
    pub fn blink_left(&self)  -> bool { self.left_blinker  || self.hazard }
    pub fn blink_right(&self) -> bool { self.right_blinker || self.hazard }
}

impl From<&PlaybackPoint> for VehicleState {
    fn from(pt: &PlaybackPoint) -> Self {
        let ls = &pt.lights;
        Self {
            no_light:              ls.no_light,
            daytime_running:       ls.daytime_running,
            low_beam:              ls.low_beam,
            high_beam:             ls.high_beam,
            left_blinker:          ls.left_blinker,
            right_blinker:         ls.right_blinker,
            hazard:                ls.hazard,
            brake:                 ls.brake,
            accelerating:          ls.accelerating,
            acc_engaged:           ls.acc_engaged,
            cruise_control_active: ls.cruise_control_active,
            speed_limiter_active:  ls.speed_limiter_active,
            mac:                   pt.mac.clone(),
            speed_kmh:             pt.speed_kmh,
            heading_deg:           pt.heading_deg,
            drive_direction:       pt.drive_direction,
        }
    }
}

// offline vehicle

#[derive(Clone, PartialEq)]
struct OfflineVehicle {
    id: u32,
    label: String,
    macs: Vec<String>,
}

fn derive_vehicle_ids(packets: &[ParsedPacket]) -> Vec<OfflineVehicle> {
    let mut ego_analyzer = EgoMac::new(10_000, 5);
    for pkt in packets {
        ego_analyzer.insert_measurement(pkt.timestamp_ms, pkt.mac.clone(), pkt.rssi);
    }
    let ego_macs: Vec<String> = ego_analyzer.evaluate().iter().map(|s| s.mac.clone()).collect();

    let mut tracker = VehicleTracker::new();
    tracker.set_ego_macs(ego_macs.iter().cloned());
    for pkt in packets {
        let speed_kmh = if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            cam.speed_kmh
        } else {
            pkt.gnw_info.as_ref().and_then(|g| {
                if g.speed >= 0x7FFF { None } else { Some(g.speed as f64 * SPEED_SCALE) }
            })
        };
        let cam = pkt.payload.as_ref().and_then(|p| {
            if let ItsPayload::Cam(cam) = p { Some(cam.as_ref()) } else { None }
        });
        tracker.insert_packet(PacketInfo {
            mac: pkt.mac.clone(),
            timestamp_ms: pkt.timestamp_ms,
            lat: pkt.gnw_info.as_ref().map(|g| g.latitude  as f64 * LAT_LON_SCALE),
            lon: pkt.gnw_info.as_ref().map(|g| g.longitude as f64 * LAT_LON_SCALE),
            pos_confidence_m:  cam.and_then(|c| c.pos_confidence_m),
            speed_kmh,
            spd_conf:          cam.and_then(|c| c.speed_confidence_ms),
            heading_deg: pkt.gnw_info.as_ref().and_then(|g| {
                if g.heading >= 3601 { None } else { Some(g.heading as f64 * HEADING_SCALE) }
            }),
            hdg_conf:          cam.and_then(|c| c.heading_confidence_deg),
            yaw_rate:          cam.and_then(|c| c.yaw_rate),
            yaw_conf:          cam.and_then(|c| c.yaw_rate_confidence_deg_s),
            accel:             cam.and_then(|c| c.longitudinal_accel),
            brake:  cam.and_then(|c| c.accel_control.as_ref()).map(|a| a.brake_pedal_active),
            gas:    cam.and_then(|c| c.accel_control.as_ref()).map(|a| a.gas_pedal_active),
            curvature:  cam.and_then(|c| c.curvature),
            v_len:      cam.and_then(|c| c.vehicle_length_m),
            v_wid:      cam.and_then(|c| c.vehicle_width_m),
            frame_seq:  pkt.frame_seq,
        });
    }

    let mut result: Vec<OfflineVehicle> = tracker
        .iter_vehicles()
        .map(|(vid, macs)| OfflineVehicle {
            id: vid,
            label: format!("Vehicle {}", vid),
            macs: macs.to_vec(),
        })
        .collect();

    if !ego_macs.is_empty() {
        result.push(OfflineVehicle { id: 0, label: "Ego".to_string(), macs: ego_macs });
    }
    result.sort_by_key(|v| v.id);
    result
}

// playback helpers

/// Resets playback to the first timestamp and stops the loop
fn restart_playback(
    mut playing: Signal<bool>,
    timeline: Signal<Vec<PlaybackPoint>>,
    mut time_ms: Signal<i64>,
) {
    playing.set(false);
    let start = timeline.read().first().map(|p| p.timestamp_ms).unwrap_or(0);
    time_ms.set(start);
}

/// Spawns the async tick loop that advances `playback_time_ms` at the given speed.
///
/// `my_gen` is the generation captured at spawn time. The loop exits immediately when `gen` has advanced beyond `my_gen`, 
// ensuring only the most recently spawned loop stays alive when Play is pressed in rapid succession.
fn spawn_playback_loop(
    mut playing: Signal<bool>,
    speed: Signal<u32>,
    timeline: Signal<Vec<PlaybackPoint>>,
    mut time_ms: Signal<i64>,
    my_gen: u32,
    gen: Signal<u32>,
) {
    spawn(async move {
        loop {
            async_std::task::sleep(Duration::from_millis(PLAYBACK_TICK_MS)).await;
            if !*playing.read() || *gen.read() != my_gen { break; }

            let advance = *speed.read() as i64 * PLAYBACK_TICK_MS as i64;
            let end_ts = {
                let tl = timeline.read();
                if tl.is_empty() { break; }
                tl.last().unwrap().timestamp_ms
            };
            let new_time = (*time_ms.read() + advance).min(end_ts);
            time_ms.set(new_time);

            if new_time >= end_ts {
                playing.set(false);
                break;
            }
        }
    });
}

// frame interpolation

/// Linearly interpolates between adjacent timeline points at time `t`.
/// Returns the active segment's index, interpolated `(lon, lat)`, and the `VehicleState` snapshot taken from that segment's start point.
fn interp_frame(timeline: &[PlaybackPoint], t: i64) -> Option<(usize, f64, f64, VehicleState)> {
    if timeline.is_empty() { return None; }
    let raw_idx = timeline.partition_point(|p| p.timestamp_ms <= t);
    let idx     = raw_idx.saturating_sub(1).min(timeline.len() - 1);
    let pt      = &timeline[idx];
    let (lon, lat) = if idx + 1 < timeline.len() {
        let next = &timeline[idx + 1];
        let span = (next.timestamp_ms - pt.timestamp_ms) as f64;
        let frac = if span > 0.0 {
            ((t - pt.timestamp_ms) as f64 / span).clamp(0.0, 1.0)
        } else { 0.0 };
        (pt.lon + frac * (next.lon - pt.lon), pt.lat + frac * (next.lat - pt.lat))
    } else {
        (pt.lon, pt.lat)
    };
    Some((idx, lon, lat, VehicleState::from(pt)))
}

/// Advances the online render clock by one tick, clamped to the latest packet timestamp. Snaps forward if the clock fell too far behind (e.g. vehicle switch or throttled tab).
fn advance_online_clock(cur: i64, latest_ts: i64) -> i64 {
    let lower = latest_ts.saturating_sub(ONLINE_LAG_MAX_MS);
    if cur == 0 || cur < lower {
        return latest_ts.saturating_sub(ONLINE_LAG_INIT_MS);
    }
    (cur + ONLINE_TICK_MS as i64).min(latest_ts)
}

// page

#[component]
pub fn LiveView() -> Element {
    let connection            = use_context::<ConnectionService>();
    let mut selected_vehicle  = use_signal(|| String::new());
    let mut state             = use_signal(VehicleState::default);
    let mut mode              = use_signal(|| LiveMode::Demo);
    let mut playing           = use_signal(|| false);
    let mut speed             = use_signal(|| 1u32);
    let mut show_picker       = use_signal(|| false);
    let mut show_ego_macs     = use_signal(|| false);
    let mut auto_center       = use_signal(|| true);
    let mut offline_packets:  Signal<Vec<ParsedPacket>>   = use_signal(Vec::new);
    let mut offline_vehicles: Signal<Vec<OfflineVehicle>> = use_signal(Vec::new);

    // layback signals
    let mut playback_timeline: Signal<Vec<PlaybackPoint>> = use_signal(Vec::new);
    let mut playback_map_pts:  Signal<Vec<(f64, f64)>>    = use_signal(Vec::new);
    let mut playback_colors:   Signal<Vec<String>>        = use_signal(Vec::new);
    let mut playback_time_ms:  Signal<i64>                = use_signal(|| 0_i64);
    // incremented on every Play press so stale spwaned loops self-terminate
    let mut playback_gen:      Signal<u32>                = use_signal(|| 0_u32);

    //drain coroutine: sends (idx, lon, lat, hdg) to the WebView, drops stale frames
    let frame_ch = use_coroutine(|mut rx: UnboundedReceiver<(usize, f64, f64, f64)>| async move {
        while let Some(mut latest) = rx.next().await {
            loop {
                match rx.try_next() {
                    Ok(Some(f)) => latest = f,
                    _ => break,
                }
            }
            let (idx, lon, lat, hdg) = latest;
            let js = format!("window._citesNextFrame=[{lon:.6},{lat:.6},{idx},{hdg:.2}];");
            let _ = document::eval(&js).await;
        }
    });

    // online mode: re-parse the temp_ pcapng file on a tight interval and refresh vehicles/packets
    // TODO: push-based update instead of polling
    let mut online_packets:  Signal<Vec<ParsedPacket>>   = use_signal(Vec::new);
    let mut online_vehicles: Signal<Vec<OfflineVehicle>> = use_signal(Vec::new);

    use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            async_std::task::sleep(Duration::from_millis(ONLINE_POLL_MS)).await;
            if *mode.read() != LiveMode::Online {
                // Clear stale data when leaving Online mode.
                if !online_packets.read().is_empty()  { online_packets.set(vec![]); }
                if !online_vehicles.read().is_empty() { online_vehicles.set(vec![]); }
                continue;
            }
            let path = match connection.current_capture_path.read().clone() {
                Some(p) => p,
                None => continue,
            };
            let result = async_std::task::spawn_blocking(move || {
                core_logic::pcap_parser::PcapParser::parse_file(&path)
            }).await;
            let Ok(pkts) = result else { continue };
            if pkts.is_empty() { continue; }
            let vehicles = derive_vehicle_ids(&pkts);
            online_vehicles.set(vehicles);
            online_packets.set(pkts);
        }
    });

    // Online timeline (with timestamps) feeds both the trajectory layer and the wall-clock-driven render tick that interpolates between packets.
    let online_timeline = use_memo(move || -> Vec<PlaybackPoint> {
        if *mode.read() != LiveMode::Online { return vec![]; }
        let sel = selected_vehicle.read().clone();
        if sel.is_empty() { return vec![]; }
        let vid: u32 = match sel.parse() { Ok(v) => v, Err(_) => return vec![] };
        let vehicles = online_vehicles.read();
        let vehicle  = match vehicles.iter().find(|v| v.id == vid) { Some(v) => v, None => return vec![] };
        let mac_set: HashSet<&str> = vehicle.macs.iter().map(String::as_str).collect();
        let pkts = online_packets.read();
        build_playback_timeline(&pkts, &mac_set)
    });

    // Trajectory points and per-MAC colors derived from the online timeline.
    // MapView's upsert path keeps layers stable as the trajectory grows.
    let online_data = use_memo(move || -> (Vec<(f64, f64)>, Vec<String>) {
        let tl = online_timeline.read();
        if tl.is_empty() { return (vec![], vec![]); }
        let sel = selected_vehicle.read().clone();
        let vid: u32 = match sel.parse() { Ok(v) => v, Err(_) => return (vec![], vec![]) };
        let vehicles = online_vehicles.read();
        let vehicle  = match vehicles.iter().find(|v| v.id == vid) { Some(v) => v, None => return (vec![], vec![]) };
        let colors: HashMap<&str, &str> = vehicle.macs.iter().enumerate()
            .map(|(i, mac)| (mac.as_str(), TRAJ_COLORS[i % TRAJ_COLORS.len()]))
            .collect();
        let pts = tl.iter().map(|p| (p.lon, p.lat)).collect();
        let cols = tl.iter()
            .map(|p| colors.get(p.mac.as_str()).copied().unwrap_or(TRAJ_COLORS[0]).to_string())
            .collect();
        (pts, cols)
    });

    // Online render clock (packet-time domain). Advances at wall-clock pace, clamped to the latest packet so we never extrapolate.
    let mut online_time_ms: Signal<i64> = use_signal(|| 0_i64);

    // Render tick: advances the online clock and dispatches an interpolated frame. Reuses `interp_frame` so motion stays smooth between packets.
    use_coroutine(move |_: UnboundedReceiver<()>| async move {
        loop {
            async_std::task::sleep(Duration::from_millis(ONLINE_TICK_MS)).await;
            if *mode.read() != LiveMode::Online { continue; }
            let tl = online_timeline.read();
            let Some(latest_ts) = tl.last().map(|p| p.timestamp_ms) else { continue; };
            let target = advance_online_clock(*online_time_ms.read(), latest_ts);
            online_time_ms.set(target);
            if let Some((idx, lon, lat, vs)) = interp_frame(&tl, target) {
                let hdg = vs.heading_deg.unwrap_or(0.0);
                frame_ch.send((idx, lon, lat, hdg));
                state.set(vs);
            }
        }
    });


    // per mac colored trajeectory lines for the map
    let trajectories = use_memo(move || {
        let selected = selected_vehicle.read();
        if selected.is_empty() { return vec![]; }
        let vid: u32 = match selected.parse() { Ok(v) => v, Err(_) => return vec![] };

        let vehicles = offline_vehicles.read();
        let vehicle  = match vehicles.iter().find(|v| v.id == vid) { Some(v) => v, None => return vec![] };
        let mac_set: HashSet<&str> = vehicle.macs.iter().map(String::as_str).collect();
        let packets  = offline_packets.read();

        build_trajectories(&packets, Some(&mac_set))
            .into_iter()
            .enumerate()
            .map(|(i, traj)| LiveTrajData {
                mac:    traj.mac,
                color:  TRAJ_COLORS[i % TRAJ_COLORS.len()].to_string(),
                points: traj.points,
            })
            .collect()
    });

    // rebuild playback timelien on vehicle / packet change
    use_effect(move || {
        if *mode.read() != LiveMode::Offline {
            playback_timeline.set(vec![]);
            playback_map_pts.set(vec![]);
            playback_colors.set(vec![]);
            return;
        }
        let sel_str = selected_vehicle.read().clone();
        if sel_str.is_empty() {
            playback_timeline.set(vec![]);
            playback_map_pts.set(vec![]);
            playback_colors.set(vec![]);
            return;
        }
        let vid: u32 = match sel_str.parse() { Ok(v) => v, Err(_) => return };
        let vehicles = offline_vehicles.read();
        let vehicle  = match vehicles.iter().find(|v| v.id == vid) { Some(v) => v, None => return };
        let mac_set: HashSet<&str> = vehicle.macs.iter().map(String::as_str).collect();
        let packets  = offline_packets.read();

        // build mac->color map from the trajectories that actually have valid GPS points, using the same sort order and filtering as the `trajectories` memo.
        // This avoids index shifts caused by MACs that are skipped (< 2 valid points)
        let trajs_for_color = build_trajectories(&packets, Some(&mac_set));
        let colors: HashMap<&str, &str> = trajs_for_color.iter()
            .enumerate()
            .map(|(i, t)| (t.mac.as_str(), TRAJ_COLORS[i % TRAJ_COLORS.len()]))
            .collect();

        let timeline = build_playback_timeline(&packets, &mac_set);
        let start_ts = timeline.first().map(|p| p.timestamp_ms).unwrap_or(0);
        let map_pts  = timeline.iter().map(|p| (p.lon, p.lat)).collect();
        let traj_colors   = timeline.iter()
            .map(|p| colors.get(p.mac.as_str()).copied().unwrap_or("#44aaff").to_string())
            .collect();

        playing.set(false);
        playback_time_ms.set(start_ts);
        playback_timeline.set(timeline);
        playback_map_pts.set(map_pts);
        playback_colors.set(traj_colors);
    });

    // current frame: interpolated position + full vehicle state
    let current_frame = use_memo(move || -> Option<(usize, f64, f64, VehicleState)> {
        interp_frame(&playback_timeline.read(), *playback_time_ms.read())
    });

    // foreign MACs visible within +/-VISIBLE_MAC_WINDOW_MS
    let vis_macs = use_memo(move || -> Vec<(String, String, f64, Option<f64>)> {
        let cur_mode = mode.read().clone();
        if cur_mode == LiveMode::Demo { return vec![]; }

        let is_online = cur_mode == LiveMode::Online;
        let t = if is_online {
            online_packets.read().last().map(|p| p.timestamp_ms).unwrap_or(0)
        } else {
            *playback_time_ms.read()
        };

        let packets = if is_online { online_packets.read() } else { offline_packets.read() };
        let vehicles = if is_online { online_vehicles.read() } else { offline_vehicles.read() };
        let show_ego = *show_ego_macs.read();

        let selected_vid: u32 = selected_vehicle.read().parse().unwrap_or(u32::MAX);
        let selected_macs: HashSet<&str> = vehicles.iter()
            .find(|v| v.id == selected_vid)
            .map(|v| v.macs.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let ego_macs: HashSet<&str> = vehicles.iter()
            .find(|v| v.id == 0)
            .map(|v| v.macs.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let labels: HashMap<&str, String> = vehicles.iter()
            .flat_map(|v| {
                let label = if v.id == 0 { "Ego".to_string() } else { v.id.to_string() };
                v.macs.iter().map(move |m| (m.as_str(), label.clone()))
            })
            .collect();

        // find the selected vehicle's position around `t` to compute distance -> just look for the closest packet in time for any of the selected_macs
        let mut selected_lat_lon = None;
        let mut min_dt_selected = i64::MAX;
        for pkt in packets.iter() {
            if selected_macs.contains(pkt.mac.as_str()) {
                let dt = (pkt.timestamp_ms - t).abs();
                if dt <= VISIBLE_MAC_WINDOW_MS && dt < min_dt_selected {
                    if let Some(g) = &pkt.gnw_info {
                        selected_lat_lon = Some((g.latitude as f64 * LAT_LON_SCALE, g.longitude as f64 * LAT_LON_SCALE));
                        min_dt_selected = dt;
                    }
                }
            }
        }

        // we want the most recent packet per mac within the window to get its rssi and location
        let mut pkt_map = HashMap::new();
        for pkt in packets.iter() {
            let dt = (pkt.timestamp_ms - t).abs();
            if dt <= VISIBLE_MAC_WINDOW_MS {
                let entry = pkt_map.entry(pkt.mac.clone()).or_insert_with(|| pkt.clone());
                if (pkt.timestamp_ms - t).abs() < (entry.timestamp_ms - t).abs() {
                    *entry = pkt.clone();
                }
            }
        }

        let mut result: Vec<(String, String, f64, Option<f64>)> = pkt_map.into_iter()
            .filter(|(mac, _)| !selected_macs.contains(mac.as_str()))
            .filter(|(mac, _)| show_ego || !ego_macs.contains(mac.as_str()))
            .map(|(mac, pkt)| {
                let label = labels.get(mac.as_str())
                    .cloned().unwrap_or_else(|| "?".to_string());
                
                let distance = if let (Some((sel_lat, sel_lon)), Some(g)) = (selected_lat_lon, &pkt.gnw_info) {
                    let pkt_lat = g.latitude as f64 * LAT_LON_SCALE;
                    let pkt_lon = g.longitude as f64 * LAT_LON_SCALE;
                    Some(core_logic::tracking_warning::haversine_m(sel_lat, sel_lon, pkt_lat, pkt_lon))
                } else {
                    None
                };

                (mac, label, pkt.rssi, distance)
            })
            .collect();
        result.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        result
    });

    // timestamp displayed next to the TRAJECTORY heading (offline mode only) kept as a plain Signal updated inside the frame effect below so the
    // timestamp never causes a render cycle ahead of the position signals
    let mut current_ts_str: Signal<String> = use_signal(String::new);

    // Trajectory seek: poll JS global set by map click handler window._citesSeekClick` is written by the MapLibre hit-layer click
    // handler in map_view.rs and cleared here after each read
    use_coroutine(move |_: UnboundedReceiver<()>| async move {
        // 150 ms keeps the seek responsive without overlaping successive evals.
        const POLL_MS: u64 = 150;
        loop {
            async_std::task::sleep(Duration::from_millis(POLL_MS)).await;
            if *mode.read() != LiveMode::Offline { continue; }
            if playback_timeline.read().is_empty() { continue; }

            // atomically read and clear the JS global so each click is consumed once
            let mut eval = document::eval(
                "var c=window._citesSeekClick;\
                 window._citesSeekClick=null;\
                 dioxus.send(c?JSON.stringify(c):'null');"
            );
            let Ok(s) = eval.recv::<String>().await else { continue };
            if s == "null" || s.is_empty() { continue; }
            let Ok(coords) = serde_json::from_str::<[f64; 2]>(&s) else { continue };
            let [lon, lat] = coords;

            // nearest-nighbor via squared Euclidean distance in degrees -
            // sufficient for sub-kilometer trajectory segments
            let tl = playback_timeline.read();
            if let Some(pt) = tl.iter().min_by(|a, b| {
                let sq_dist = |p: &&crate::trajectory::PlaybackPoint| {
                    (p.lon - lon).powi(2) + (p.lat - lat).powi(2)
                };
                sq_dist(a).partial_cmp(&sq_dist(b)).unwrap_or(std::cmp::Ordering::Equal)
            }) {
                playback_time_ms.set(pt.timestamp_ms);
            }
        }
    });

    // apply current frame to car view and map
    // both updates happen in the same effect so the map and car are allways in sync
    use_effect(move || {
        if *mode.read() != LiveMode::Offline { return; }
        if playback_map_pts.read().is_empty() { return; }
        if let Some((idx, lon, lat, vs)) = current_frame() {
            let hdg = vs.heading_deg.unwrap_or(0.0);
            frame_ch.send((idx, lon, lat, hdg));
            state.set(vs);

            let use_gnw  = *connection.ts_use_gnw.read();
            let as_unix  = *connection.ts_unix_format.read();
            let unix_ms = if use_gnw {
                playback_timeline.read().get(idx)
                    .and_then(|pt| pt.gnw_timestamp_ms.map(|tst| gnw_tst_to_unix_ms(tst, pt.timestamp_ms)))
                    .unwrap_or_else(|| *playback_time_ms.read())
            } else {
                *playback_time_ms.read()
            };
            current_ts_str.set(format_unix_ms(unix_ms, as_unix));
        }
    });

    // MapView props - position/heading are delivered via frame_ch, not props
    let (map_trajs, map_pb_pts, map_pb_colors) =
        if *mode.read() == LiveMode::Online {
            let (traj, cols) = online_data();
            (vec![], traj, cols)
        } else {
            (trajectories(), playback_map_pts(), playback_colors())
        };

    rsx! {
        document::Link { rel: "stylesheet", href: LIVE_CSS }

        div { class: "live-container",
            VehicleHeader {
                selected_vehicle,
                mode: mode(),
                offline_vehicles: offline_vehicles(),
                online_vehicles: online_vehicles(),
            }

            div { class: "live-controls",
                span { class: "controls-label", {mode.read().label()} }

                {
                    match *mode.read() {
                        LiveMode::Demo => rsx! {
                            DemoButtons { state }
                        },
                        LiveMode::Online => rsx! {
                            button {
                                class: if *show_ego_macs.read() { "ctrl-btn active" } else { "ctrl-btn" },
                                onclick: move |_| show_ego_macs.with_mut(|v| *v = !*v),
                                "Ego MACs"
                            }
                            {
                                let stats = connection.pcap_stats.read();
                                let (traj, _) = online_data();
                                let traj_len = traj.len();
                                let (cur_lon, cur_lat) = traj.last().copied().unwrap_or((0.0, 0.0));
                                rsx! {
                                    span { class: "controls-label",
                                        "Pkts: {stats.total_packets} | Traj: {traj_len} pts | GPS: ({cur_lat:.4}, {cur_lon:.4})"
                                    }
                                }
                            }
                        },
                        LiveMode::Offline => rsx! {
                            button {
                                class: "ctrl-btn",
                                onclick: move |_| restart_playback(playing, playback_timeline, playback_time_ms),
                                "Restart"
                            }
                            button {
                                class: if *playing.read() { "ctrl-btn active" } else { "ctrl-btn" },
                                onclick: move |_| {
                                    if *playing.read() {
                                        playing.set(false);
                                    } else {
                                        // Jump back to start if already at end.
                                        let at_end = {
                                            let tl = playback_timeline.read();
                                            !tl.is_empty()
                                                && *playback_time_ms.read() >= tl.last().unwrap().timestamp_ms
                                        };
                                        if at_end {
                                            restart_playback(playing, playback_timeline, playback_time_ms);
                                        }
                                        let gen = playback_gen.read().wrapping_add(1);
                                        playback_gen.set(gen);
                                        playing.set(true);
                                        spawn_playback_loop(
                                            playing,
                                            speed,
                                            playback_timeline,
                                            playback_time_ms,
                                            gen,
                                            playback_gen,
                                        );
                                    }
                                },
                                if *playing.read() {
                                    "Pause"
                                } else {
                                    "Play"
                                }
                            }
                            div { class: "speed-control",
                                span { class: "speed-label", "Speed: {speed}x" }
                                input {
                                    r#type: "range",
                                    min: "1",
                                    max: "100",
                                    value: "{speed}",
                                    class: "speed-slider",
                                    oninput: move |e| {
                                        if let Ok(v) = e.value().parse::<u32>() {
                                            speed.set(v);
                                        }
                                    },
                                }
                            }
                            button {
                                class: if *show_ego_macs.read() { "ctrl-btn active" } else { "ctrl-btn" },
                                onclick: move |_| show_ego_macs.with_mut(|v| *v = !*v),
                                "Ego MACs"
                            }
                            button { class: "ctrl-btn", onclick: move |_| show_picker.set(true), "Open File" }
                        },
                    }
                }

                div { class: "controls-spacer" }

                button {
                    class: if *auto_center.read() { "ctrl-btn autocenter-btn active" } else { "ctrl-btn autocenter-btn" },
                    onclick: move |_| {
                        let enabling = !*auto_center.read();
                        auto_center.set(enabling);
                        let js = if enabling {
                            "window._liveAutoCenter=true;\
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             if(window._liveLast&&window._liveMap){\
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             var p=window._liveLast;\
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             window._liveMap.easeTo({center:[p.lon,p.lat],bearing:p.hdg||0,duration:400});}"
                        } else {
                            "window._liveAutoCenter=false;"
                        };
                        spawn(async move {
                            let _ = document::eval(js).await;
                        });
                    },
                    "Auto-Center"
                }
                button {
                    class: "ctrl-btn mode-btn",
                    onclick: move |_| {
                        let next = mode.read().next();
                        mode.set(next);
                        selected_vehicle.set(String::new());
                    },
                    "Change Mode"
                }
            }

            div { class: "live-content",
                div { class: "car-panel",
                    h3 { class: "panel-title", "Vehicle Status" }
                    {
                        // prefer the MAC address from the latest parsed packet (populated for both Offline and Online modes via VehicleState::from(PlaybackPoint))
                        // fall back to the vehicle ID string if no packet has been seen yet
                        let mac = state.read().mac.clone();
                        let label = if !mac.is_empty() { mac } else { selected_vehicle.read().clone() };
                        if !label.is_empty() {
                            rsx! {
                                div { class: "car-mac-label", "{label}" }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                    CarView { state: state() }
                    if *mode.read() != LiveMode::Demo {
                        div { class: "visible-macs-section",
                            div { class: "visible-macs-header", "Visible MACs (ID, dBm, m)" }
                            div { class: "visible-macs-list",
                                if vis_macs().is_empty() {
                                    div { class: "visible-macs-empty", "None" }
                                }
                                for (mac , label , rssi , dist) in vis_macs() {
                                    div { class: "visible-mac-entry",
                                        span { class: "visible-mac-addr", "{mac}" }
                                        if let Some(d) = dist {
                                            span { class: "visible-mac-id",
                                                " ({label}, {rssi:.1}, {d:.1})"
                                            }
                                        } else {
                                            span { class: "visible-mac-id", " ({label}, {rssi:.1}, -)" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "map-panel",
                    div { class: "panel-title-row",
                        h3 { class: "panel-title", "Trajectory" }
                        if *mode.read() == LiveMode::Offline {
                            span { class: "panel-ts", "{current_ts_str}" }
                        }
                    }
                    MapView {
                        trajectories: map_trajs,
                        playback_map_pts: map_pb_pts,
                        playback_colors: map_pb_colors,
                    }
                }
            }
        }

        if *show_picker.read() {
            SourcePicker {
                on_dismiss: move |_| show_picker.set(false),
                on_loaded: move |pkts: Vec<ParsedPacket>| {
                    let vehicles = derive_vehicle_ids(&pkts);
                    offline_vehicles.set(vehicles);
                    selected_vehicle.set(String::new());
                    *offline_packets.write() = pkts;
                    show_picker.set(false);
                },
            }
        }
    }
}

// sub-components

#[component]
fn VehicleHeader(
    selected_vehicle: Signal<String>,
    mode: LiveMode,
    offline_vehicles: Vec<OfflineVehicle>,
    online_vehicles: Vec<OfflineVehicle>,
) -> Element {
    rsx! {
        div { class: "live-header",
            h2 { class: "live-title", "Live Monitoring" }
            div { class: "live-vehicle-select",
                label { r#for: "vehicle-dropdown", "Vehicle:" }
                select {
                    id: "vehicle-dropdown",
                    class: "vehicle-dropdown",
                    onchange: move |e| selected_vehicle.set(e.value()),
                    option {
                        value: "",
                        disabled: true,
                        selected: selected_vehicle().is_empty(),
                        "-- Select vehicle --"
                    }
                    match mode {
                        LiveMode::Offline => rsx! {
                            for v in offline_vehicles.iter() {
                                option {
                                    key: "{v.id}",
                                    value: "{v.id}",
                                    selected: selected_vehicle() == v.id.to_string(),
                                    "{v.label}"
                                }
                            }
                        },
                        LiveMode::Online => rsx! {
                            for v in online_vehicles.iter() {
                                {
                                    let val = v.id.to_string();
                                    rsx! {
                                        option { key: "{v.id}", value: "{val}", selected: selected_vehicle() == val, "{v.label}" }
                                    }
                                }
                            }
                        },
                        LiveMode::Demo => rsx! {
                            for v in DEMO_VEHICLES {
                                option { value: *v, "{v}" }
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn DemoButtons(state: Signal<VehicleState>) -> Element {
    rsx! {
        button {
            class: if state().no_light { "ctrl-btn active" } else { "ctrl-btn" },
            onclick: move |_| state.with_mut(|s| s.no_light = !s.no_light),
            "No Light"
        }
        button {
            class: if state().daytime_running { "ctrl-btn active" } else { "ctrl-btn" },
            onclick: move |_| state.with_mut(|s| s.daytime_running = !s.daytime_running),
            "DRL"
        }
        button {
            class: if state().low_beam { "ctrl-btn active" } else { "ctrl-btn" },
            onclick: move |_| state.with_mut(|s| s.low_beam = !s.low_beam),
            "Low Beam"
        }
        button {
            class: if state().high_beam { "ctrl-btn active" } else { "ctrl-btn" },
            onclick: move |_| state.with_mut(|s| s.high_beam = !s.high_beam),
            "High Beam"
        }
        button {
            class: if state().left_blinker { "ctrl-btn blinker-btn active" } else { "ctrl-btn blinker-btn" },
            onclick: move |_| {
                state
                    .with_mut(|s| {
                        s.left_blinker = !s.left_blinker;
                        if s.left_blinker {
                            s.hazard = false;
                        }
                    })
            },
            "Left"
        }
        button {
            class: if state().right_blinker { "ctrl-btn blinker-btn active" } else { "ctrl-btn blinker-btn" },
            onclick: move |_| {
                state
                    .with_mut(|s| {
                        s.right_blinker = !s.right_blinker;
                        if s.right_blinker {
                            s.hazard = false;
                        }
                    })
            },
            "Right"
        }
        button {
            class: if state().hazard { "ctrl-btn hazard-btn active" } else { "ctrl-btn hazard-btn" },
            onclick: move |_| {
                state
                    .with_mut(|s| {
                        s.hazard = !s.hazard;
                        if s.hazard {
                            s.left_blinker = false;
                            s.right_blinker = false;
                        }
                    })
            },
            "Hazard"
        }
        button {
            class: if state().brake { "ctrl-btn brake-btn active" } else { "ctrl-btn brake-btn" },
            onclick: move |_| state.with_mut(|s| s.brake = !s.brake),
            "Brake"
        }
        button {
            class: if state().accelerating { "ctrl-btn accel-btn active" } else { "ctrl-btn accel-btn" },
            onclick: move |_| state.with_mut(|s| s.accelerating = !s.accelerating),
            "Accel"
        }
    }
}

// timestamp formatting helpers

const MS_PER_SEC:   i64 = 1_000;
const SECS_PER_MIN: i64 = 60;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;

/// Converts a Unix-epoch day count to `(year, month, day)`.
///
/// Uses Howard Hinnant's civil-calendar algorithm:
/// <https://howardhinnant.github.io/date_algorithms.html>
fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z    = days + 719_468;
    let era  = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe  = z - era * 146_097;
    let yoe  = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y    = yoe + era * 400;
    let doy  = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp   = (5 * doy + 2) / 153;
    let d    = doy - (153 * mp + 2) / 5 + 1;
    let m    = if mp < 10 { mp + 3 } else { mp - 9 };
    let y    = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Formats a Unix-millisecond timestamp as either a raw integer or `hh:mm:ss dd.mm.yyyy`.
fn format_unix_ms(unix_ms: i64, as_unix: bool) -> String {
    if as_unix {
        return unix_ms.to_string();
    }
    let unix_secs   = unix_ms.div_euclid(MS_PER_SEC);
    let secs_in_day = unix_secs.rem_euclid(SECS_PER_DAY);
    let h = secs_in_day / SECS_PER_HOUR;
    let m = (secs_in_day % SECS_PER_HOUR) / SECS_PER_MIN;
    let s = secs_in_day % SECS_PER_MIN;
    let (yr, mo, d) = days_to_ymd(unix_secs.div_euclid(SECS_PER_DAY));
    format!("{h:02}:{m:02}:{s:02} {d:02}.{mo:02}.{yr}")
}

/// Unwraps a GNW LPV TST value to a Unix-millisecond timestamp.
///
/// TST is a u32 counting TAI milliseconds since 2004-01-01 00:00:00 UTC,
/// wrapping every 2^32 ms (~49.7 days). `approx_unix_ms` is used to
/// disambiguate which period the timestamp belongs to (ETSI EN 302 636-4-1).
fn gnw_tst_to_unix_ms(gnw_tst: u32, approx_unix_ms: i64) -> i64 {
    // 2004-01-01T00:00:00Z expressed as Unix ms (ETSI EN 302 636-4-1, clause 6.4).
    const ITS_EPOCH_MS: i64 = 1_072_915_200_000;
    const WRAP_MS: i64      = 1_i64 << 32; // u32 wrap period
    let tst_ms   = gnw_tst as i64;
    let its_ms   = approx_unix_ms - ITS_EPOCH_MS;
    let period_n = (its_ms - tst_ms).div_euclid(WRAP_MS);
    let c1       = tst_ms + period_n * WRAP_MS;
    let c2       = tst_ms + (period_n + 1) * WRAP_MS;
    // pick the candidate closer to the anchor (PCAP timestamp)
    let unwrapped = if (c1 - its_ms).abs() <= (c2 - its_ms).abs() { c1 } else { c2 };
    unwrapped + ITS_EPOCH_MS
}
