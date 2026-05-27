use std::collections::{HashMap, HashSet};
use core_logic::pcap_parser::ParsedPacket;
use core_logic::parser::decoder::ItsPayload;
use core_logic::vehicle_tracker::{LAT_LON_SCALE, SPEED_SCALE};

const LAT_SENTINEL: f64 = 89.9;
const LON_SENTINEL: f64 = 179.9;
const MAX_SPEED_KMH: f64 = 300.0;

// static trajectory (offline)

/// One MAC address with its filtered, cleaned trajectory.
/// Points are in GeoJSON order: (longitude, latitude).
#[derive(Clone, PartialEq)]
pub struct MacTrajectory {
    pub mac: String,
    pub points: Vec<(f64, f64)>,
}

fn haversine_km(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let r = 6_371.0_f64;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

/// Returns `true` if the GPS point should be rejected.
fn is_invalid(lat: f64, lon: f64) -> bool {
    lat.abs() > LAT_SENTINEL || lon.abs() > LON_SENTINEL || (lat == 0.0 && lon == 0.0)
}

/// Returns `(lat, lon)` for a packet, prefering the C-ITS CAM payload position
/// over the GNW layer position.
fn position_from_pkt(pkt: &ParsedPacket) -> Option<(f64, f64)> {
    if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
        if let (Some(lat), Some(lon)) = (cam.latitude, cam.longitude) {
            if !is_invalid(lat, lon) {
                return Some((lat, lon));
            }
        }
    }
    let g = pkt.gnw_info.as_ref()?;
    let lat = g.latitude as f64 * LAT_LON_SCALE;
    let lon = g.longitude as f64 * LAT_LON_SCALE;
    if is_invalid(lat, lon) { return None; }
    Some((lat, lon))
}

/// Builds per-MAC trajectories from a packet slice.
///
/// Filters: ITS-G5 sentinels, zero-island, timestamp sort, haversine speed outliers.
/// Returns trajectories sorted by MAC for deterministic colour assignment.
pub fn build_trajectories(
    packets: &[ParsedPacket],
    mac_filter: Option<&HashSet<&str>>,
) -> Vec<MacTrajectory> {
    let mut by_mac: HashMap<String, Vec<(i64, f64, f64)>> = HashMap::new();

    for pkt in packets {
        if let Some(filter) = mac_filter {
            if !filter.contains(pkt.mac.as_str()) {
                continue;
            }
        }
        let Some((lat, lon)) = position_from_pkt(pkt) else { continue };
        by_mac.entry(pkt.mac.clone()).or_default().push((pkt.timestamp_ms, lon, lat));
    }

    let mut macs: Vec<String> = by_mac.keys().cloned().collect();
    macs.sort();

    let mut result = Vec::new();
    for mac in macs {
        let mut raw = by_mac.remove(&mac).unwrap();
        raw.sort_by_key(|(ts, _, _)| *ts);

        let mut points: Vec<(f64, f64)> = Vec::with_capacity(raw.len());
        let mut prev: Option<(i64, f64, f64)> = None;
        for (ts, lon, lat) in raw {
            if let Some((prev_ts, prev_lon, prev_lat)) = prev {
                let dt_h = (ts - prev_ts) as f64 / 3_600_000.0;
                if dt_h > 0.0 && haversine_km(prev_lon, prev_lat, lon, lat) / dt_h > MAX_SPEED_KMH {
                    continue;
                }
            }
            prev = Some((ts, lon, lat));
            points.push((lon, lat));
        }

        if points.len() < 2 { continue; }
        result.push(MacTrajectory { mac, points });
    }
    result
}

// Playback timeline 

/// The light + motion state at a single point in the playback timeline.
#[derive(Clone, PartialEq, Debug)]
pub struct PlaybackLightState {
    pub no_light: bool,
    pub daytime_running: bool,
    pub low_beam: bool,
    pub high_beam: bool,
    pub left_blinker: bool,
    pub right_blinker: bool,
    pub hazard: bool,
    pub brake: bool,
    pub accelerating: bool,
    pub acc_engaged: bool,
    pub cruise_control_active: bool,
    pub speed_limiter_active: bool,
    pub reverse_light: bool,
}

impl Default for PlaybackLightState {
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
            reverse_light: false,
        }
    }
}

/// Direction of travel from the CAM `DriveDirection` field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriveDirection {
    Forward,
    Backward,
    Unavailable,
}

impl DriveDirection {
    /// Maps the raw CAM `DriveDirection` value (0/1/2) to the typed enum.
    /// Returns `None` for any value outside the standard range.
    pub fn from_raw(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Forward),
            1 => Some(Self::Backward),
            2 => Some(Self::Unavailable),
            _ => None,
        }
    }
}

/// A single sample in the playback timeline: GPS position + vehicle state.
/// Points are in GeoJSON order: (longitude, latitude).
#[derive(Clone, PartialEq)]
pub struct PlaybackPoint {
    pub timestamp_ms: i64,
    pub lon: f64,
    pub lat: f64,
    pub mac: String,
    pub speed_kmh: Option<f64>,
    /// Compass heading in degrees (0 = north, 90 = east).
    pub heading_deg: Option<f64>,
    pub drive_direction: Option<DriveDirection>,
    pub lights: PlaybackLightState,
    /// GNW LPV TST: TAI ms since 2004-01-01 UTC, mod 2^32 (ETSI EN 302 636-4-1 Table 4).
    pub gnw_timestamp_ms: Option<u32>,
}

fn lights_from_cam(cam: &core_logic::parser::types::DecodedCam, prev: &PlaybackLightState) -> PlaybackLightState {
    let accel = cam.accel_control.as_ref();
    match &cam.exterior_lights {
        Some(e) => {
            let hazard = e.left_turn && e.right_turn;
            PlaybackLightState {
                no_light: false,
                daytime_running: e.daytime_running,
                low_beam: e.low_beam,
                high_beam: e.high_beam,
                left_blinker: e.left_turn && !hazard,
                right_blinker: e.right_turn && !hazard,
                hazard,
                brake: accel.map_or(prev.brake, |a| a.brake_pedal_active),
                accelerating: accel.map_or(prev.accelerating, |a| a.gas_pedal_active),
                acc_engaged: accel.map_or(prev.acc_engaged, |a| a.acc_engaged),
                cruise_control_active: accel.map_or(prev.cruise_control_active, |a| a.cruise_control_active),
                speed_limiter_active: accel.map_or(prev.speed_limiter_active, |a| a.speed_limiter_active),
                reverse_light: e.reverse_light,
            }
        }
        None => PlaybackLightState {
            brake: accel.map_or(prev.brake, |a| a.brake_pedal_active),
            accelerating: accel.map_or(prev.accelerating, |a| a.gas_pedal_active),
            acc_engaged: accel.map_or(prev.acc_engaged, |a| a.acc_engaged),
            cruise_control_active: accel.map_or(prev.cruise_control_active, |a| a.cruise_control_active),
            speed_limiter_active: accel.map_or(prev.speed_limiter_active, |a| a.speed_limiter_active),
            ..prev.clone()
        },
    }
}

/// Builds a chronological playback timeline for a vehicle (all its MACs merged).
///
/// Each `PlaybackPoint` carries the GPS position at that moment plus the most
/// recent light/pedal state seen in any preceding CAM packet from the same vehicle.
///
/// Only packets belonging to `mac_filter` are considered.  The resulting
/// timeline is sorted by timestamp and has haversine-speed outliers removed.
pub fn build_playback_timeline(
    packets: &[ParsedPacket],
    mac_filter: &HashSet<&str>,
) -> Vec<PlaybackPoint> {
    // Collect and sort all relevant packets by timestamp.
    let mut sorted: Vec<&ParsedPacket> = packets
        .iter()
        .filter(|p| mac_filter.contains(p.mac.as_str()))
        .collect();
    sorted.sort_by_key(|p| p.timestamp_ms);

    let mut result: Vec<PlaybackPoint> = Vec::new();
    let mut last_lights = PlaybackLightState::default();
    let mut last_spd: Option<f64> = None;
    let mut last_hdg: Option<f64> = None;
    let mut last_dir: Option<DriveDirection> = None;
    let mut prev_gps: Option<(i64, f64, f64)> = None;

    for pkt in &sorted {
        // Update state from CAM packets.
        if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            last_lights = lights_from_cam(cam, &last_lights);
            if cam.speed_kmh.is_some()  { last_spd = cam.speed_kmh; }
            if cam.heading_deg.is_some() { last_hdg = cam.heading_deg; }
            if let Some(d) = cam.drive_direction { last_dir = DriveDirection::from_raw(d); }
        }

        // Only emit a point when a valid GPS fix is available.
        // Prefers C-ITS CAM payload position; falls back to GNW layer position.
        let Some((lat, lon)) = position_from_pkt(pkt) else { continue };

        // Haversine speed outlier rejeciton.
        if let Some((prev_ts, prev_lon, prev_lat)) = prev_gps {
            let dt_h = (pkt.timestamp_ms - prev_ts) as f64 / 3_600_000.0;
            if dt_h > 0.0 && haversine_km(prev_lon, prev_lat, lon, lat) / dt_h > MAX_SPEED_KMH {
                continue;
            }
        }
        prev_gps = Some((pkt.timestamp_ms, lon, lat));

        // Fall back to GNW speed when no CAM speed has been seen yet.
        let spd = last_spd.or_else(|| {
            pkt.gnw_info.as_ref().and_then(|g| {
                if g.speed >= 0x7FFF { None } else { Some(g.speed as f64 * SPEED_SCALE) }
            })
        });

        result.push(PlaybackPoint {
            timestamp_ms: pkt.timestamp_ms,
            lon,
            lat,
            mac: pkt.mac.clone(),
            speed_kmh: spd,
            heading_deg: last_hdg,
            drive_direction: last_dir,
            lights: last_lights.clone(),
            gnw_timestamp_ms: pkt.gnw_info.as_ref().and_then(|g| g.gnw_timestamp_ms),
        });
    }

    result
}
