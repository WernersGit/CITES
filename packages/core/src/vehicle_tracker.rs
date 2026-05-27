use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::debug;

// C-ITS unit conversions
// latitude/longitude: 1/10 microdegree -> degree = value * 1e-7
// speed: 0.01 m/s -> km/h = value * 0.036
// heading: 0.1 degree -> degree = value * 0.1
pub const LAT_LON_SCALE: f64 = 1e-7;
pub const SPEED_SCALE: f64 = 0.036;
pub const HEADING_SCALE: f64 = 0.1;

const MAX_LINK_GAP_S: f64 = 60.0;
const MAX_LINK_GAP_MOVING_S: f64 = 8.0;
const SPATIAL_HARD_REJECT_FACTOR: f64 = 2.0;
const MAX_SPEED_DELTA_KMH: f64 = 15.0;
const MAX_HEADING_DELTA_DEG_PER_S: f64 = 35.0;
const STANDSTILL_SPEED_KMH: f64 = 1.0;
const MIN_STANDSTILL_S: f64 = 2.0;
const MIN_LINK_PROBABILITY: f64 = 0.35;
const MAX_ACCEL_EXTRAPOLATION_S: f64 = 3.0;
const TERRAIN_ACCEL_THRESHOLD_MS2: f64 = 0.3;
const TERRAIN_TOLERANCE_FACTOR: f64 = 1.5;
/// Vehicle dimension tolerance for pseudonym-change linking: exact match (0.0 m).                                                                                                                
/// CAM dimensions are fixed integer LSB values; the same vehicle always reports the same integer, so any discrepancy means a different vehicle.   
/// The 1 mm epsilon absorbs floating-point rounding only.
const DIMENSION_TOLERANCE_M: f64 = 0.001;  
const MAX_PREDICTION_S: f64 = 2.0; 
/// Minimum corridor radius: GPS uncertainty floor for stationary vehicles.
const MIN_CORRIDOR_M: f64 = 20.0;
/// Fallback confidence values used when the CAM sender omits them.
const DEFAULT_POS_CONFIDENCE_M: f64 = 10.0;
const DEFAULT_HEADING_CONF_DEG: f64 = 10.0;
const DEFAULT_SPEED_CONF_MS: f64 = 1.39; // ~5 km/h
const DEFAULT_YAW_CONF_DEG_S: f64 = 5.0;
// max CAM transmit frequency per ETSI EN 302 637-2 -> better precision with calculating transmitting frequency
// and congestion control effects / just use actual frequency to restrict the sequence number gap
const CAM_MAX_FREQ_HZ: f64 = 10.0;
const SEQ_MODULO: u16 = 4096;         // 802.11 sequence number wraps at 2^12
// TODO: maybe exppose some of these as config

/// State maintained for one virtual vehicle (one physical car, possibly
/// observed across several pseudonyms). The virtual ID is the HashMap key
/// in `VehicleTracker`; it is not stored redundantly inside the entry.
#[derive(Debug, Clone)]
struct VehicleEntry {
    macs: Vec<String>,
    mac_history: HashSet<String>,
    last_mac: String,
    last_seen: Instant,
    last_ts: f64,
    // kinematic state
    lat: Option<f64>,
    lon: Option<f64>,
    end_speed_kmh: Option<f64>,
    end_heading_deg: Option<f64>,
    accel: Option<f64>,
    brake: Option<bool>,
    gas: Option<bool>,
    yaw_rate: Option<f64>,
    standstill: f64,
    // kinematic confidence intervals (CAM HighFrequencyContainer)
    pos_conf: Option<f64>,
    hdg_conf: Option<f64>,
    spd_conf: Option<f64>,
    yaw_conf: Option<f64>,
    // hard-constraint fields
    v_len: Option<f64>, // stable per trip
    v_wid: Option<f64>,  // stable per trip
    curvature: Option<i32>,        // sign preserved across pseudonym changes
    frame_seq: Option<u16>,        // 802.11 seq (12-bit), not reset on pseudonym change
    reported_new: bool,
}

/// Kinematic snapshot extracted from a single parsed packet.
pub struct PacketInfo {
    pub mac: String,
    pub timestamp_ms: i64,
    /// Position
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub pos_confidence_m: Option<f64>,
    /// Motion
    pub speed_kmh: Option<f64>,
    pub spd_conf: Option<f64>,
    pub heading_deg: Option<f64>,
    pub hdg_conf: Option<f64>,
    pub yaw_rate: Option<f64>,
    pub yaw_conf: Option<f64>,
    /// Longitudinal acceleration in m/s^2 (raw CAM value / 10). Positive = forward.
    pub accel: Option<f64>,
    pub brake: Option<bool>,
    pub gas: Option<bool>,
    /// Raw curvature value [-30000, +30000]. Positive = left, negative = right.
    pub curvature: Option<i32>,
    /// Vehicle body dimensions from the HighFrequencyContainer (stable per trip).
    pub v_len: Option<f64>,
    pub v_wid: Option<f64>,
    /// IEEE 802.11 QoS sequence number (12-bit, 0..4095). Not reset on pseudonym change.
    pub frame_seq: Option<u16>,
}

pub enum InsertResult {
    Ego,
    Known,
    NewVehicle(u32),
}

/// Online streaming vehicle tracker
///
/// Pseudonym linking combines hard constraints (dimensions, curvature sign,
/// optional 802.11 sequence-number window) with a weighted probabilistic score
/// (spatial, speed, heading, standstill).
///
/// Complexity per `insert_packet` call:
/// - Known MAC (>99 % of packets): O(1) -- two HashMap lookups.
/// - New MAC / pseudonym change:   O(N) candidate scan, O(1) scoring per candidate.
/// - `drain_lost_vehicles`:        O(N) single-pass retain.
pub struct VehicleTracker {
    /// Primary store: virtual_id -> entry. O(1) insert, lookup, and remove.
    vehicles: HashMap<u32, VehicleEntry>,
    /// Maps every known MAC address to the virtual ID of its vehicle.
    mac_to_vid: HashMap<String, u32>,
    ego_macs: HashSet<String>,
    next_id: u32,
    /// When true (default), candidate links are rejected if the 802.11 sequence
    /// number did not advance by [1, max_gap_s * CAM_MAX_FREQ_HZ] modulo 4096.
    seq_filter: bool,
}

impl VehicleTracker {
    pub fn new() -> Self {
        Self {
            vehicles: HashMap::new(),
            mac_to_vid: HashMap::new(),
            ego_macs: HashSet::new(),
            next_id: 1,
            seq_filter: true,
        }
    }

    pub fn set_ego_macs(&mut self, macs: impl IntoIterator<Item = String>) {
        self.ego_macs = macs.into_iter().collect();
    }

    /// Enable or disable the 802.11 sequence-number hard filter for pseudonym linking.
    /// Enabled by default; disable only when `PacketInfo::frame_seq` is not populated.
    pub fn set_seq_filter(&mut self, enabled: bool) {
        self.seq_filter = enabled;
    }

    /// Process one incoming packet
    ///
    /// Returns `InsertResult::NewVehicle(id)` the first time a virtual vehicle
    /// becomes visible (first MAC seen, no prior vehicle linked by scoring).
    pub fn insert_packet(&mut self, info: PacketInfo) -> InsertResult {
        if self.ego_macs.contains(&info.mac) {
            return InsertResult::Ego;
        }

        debug!(mac = %info.mac, seq = ?info.frame_seq, "frame");

        let ts = info.timestamp_ms as f64 / 1000.0;

        // known MAC
        if let Some(&vid) = self.mac_to_vid.get(&info.mac) {
            if let Some(v) = self.vehicles.get_mut(&vid) {
                v.last_seen = Instant::now();
                v.last_ts = ts;
                update_kinematic_state(v, &info);
                if let Some(spd) = info.speed_kmh {
                    if spd <= STANDSTILL_SPEED_KMH {
                        v.standstill += 0.1;
                    } else {
                        v.standstill = 0.0;
                    }
                }
                let was_new = !v.reported_new;
                v.reported_new = true;
                return if was_new { InsertResult::NewVehicle(vid) } else { InsertResult::Known };
            }
        }

        // immutable borrow: find the highest-scoring candidate vid
        let mut winner: Option<u32> = None;
        let mut best = -1.0_f64;

        for (&vid, v) in &self.vehicles {
            // no-interleaving
            if v.mac_history.contains(&info.mac) && info.mac != v.last_mac {
                continue;
            }
            // vehicle dimensions must match
            if !dimensions_consistent(v, &info) {
                continue;
            }
            // curvature sign must be preserved
            if !curvature_sign_consistent(v, &info) {
                continue;
            }

            let dt = ts - v.last_ts;
            if dt < 0.0 {
                continue;
            }
            let max_gap = if v.standstill >= MIN_STANDSTILL_S {
                MAX_LINK_GAP_S
            } else {
                MAX_LINK_GAP_MOVING_S
            };
            if dt > max_gap {
                continue;
            }
            // 802.11 seq check
            if self.seq_filter {
                if let (Some(old_seq), Some(new_seq)) = (v.frame_seq, info.frame_seq) {
                    let max_gap_frames = (max_gap * CAM_MAX_FREQ_HZ).round() as u16;
                    if !seq_advance_ok(old_seq, new_seq, max_gap_frames) {
                        continue;
                    }
                }
            }

            let prob = score_link(v, &info, dt);
            if prob > best {
                best = prob;
                winner = Some(vid);
            }
        }

        // mutable borrow: commit the link when probability is sufficient
        if best >= MIN_LINK_PROBABILITY {
            if let Some(vid) = winner {
                if let Some(v) = self.vehicles.get_mut(&vid) {
                    v.macs.push(info.mac.clone());
                    v.mac_history.insert(info.mac.clone());
                    v.last_mac = info.mac.clone();
                    v.last_seen = Instant::now();
                    v.last_ts = ts;
                    v.standstill = 0.0;
                    update_kinematic_state(v, &info);
                    self.mac_to_vid.insert(info.mac, vid);
                    let was_new = !v.reported_new;
                    v.reported_new = true;
                    return if was_new { InsertResult::NewVehicle(vid) } else { InsertResult::Known };
                }
            }
        }

        // no candidate found, register as new virtual vehicle
        let vid = self.next_id;
        self.next_id += 1;
        let mut seen_macs = HashSet::new();
        seen_macs.insert(info.mac.clone());
        self.vehicles.insert(vid, VehicleEntry {
            macs: vec![info.mac.clone()],
            mac_history: seen_macs,
            last_mac: info.mac.clone(),
            last_seen: Instant::now(),
            last_ts: ts,
            lat: info.lat,
            lon: info.lon,
            end_speed_kmh: info.speed_kmh,
            end_heading_deg: info.heading_deg,
            accel: info.accel,
            brake: info.brake,
            gas: info.gas,
            yaw_rate: info.yaw_rate,
            standstill: 0.0,
            pos_conf: info.pos_confidence_m,
            hdg_conf: info.hdg_conf,
            spd_conf: info.spd_conf,
            yaw_conf: info.yaw_conf,
            v_len: info.v_len,
            v_wid: info.v_wid,
            curvature: info.curvature,
            frame_seq: info.frame_seq,
            reported_new: true,
        });
        self.mac_to_vid.insert(info.mac, vid);
        InsertResult::NewVehicle(vid)
    }

    /// Returns an itertor over `(virtual_id, macs)` for all tracked vehicles
    pub fn iter_vehicles(&self) -> impl Iterator<Item = (u32, &[String])> {
        self.vehicles.iter().map(|(&vid, v)| (vid, v.macs.as_slice()))
    }

    /// Returns the virtual vehicle ID assigned to `mac`, if known.
    pub fn get_vid_for_mac(&self, mac: &str) -> Option<u32> {
        self.mac_to_vid.get(mac).copied()
    }

    /// Removes vehicles silent for >= `timeout_ms` ms and returns their IDs
    /// Runs in O(N) via a single `retain` pass that cleans up `mac_to_vid`
    /// in the same iteration.
    pub fn drain_lost_vehicles(&mut self, timeout_ms: u64) -> Vec<u32> {
        let tmo = std::time::Duration::from_millis(timeout_ms);
        let mut lost = Vec::new();
        // shouldn't be amny at a time in practice
        self.vehicles.retain(|&vid, v| {
            if v.last_seen.elapsed() >= tmo && v.reported_new {
                for mac in &v.macs {
                    self.mac_to_vid.remove(mac);
                }
                lost.push(vid);
                false
            } else {
                true
            }
        });
        lost
    }
}

// overwrite kinematic state with latest packet, keep first known dimensions
fn update_kinematic_state(v: &mut VehicleEntry, info: &PacketInfo) {
    v.lat                       = info.lat;
    v.lon                       = info.lon;
    v.end_speed_kmh             = info.speed_kmh;
    v.end_heading_deg           = info.heading_deg;
    v.accel                     = info.accel;
    v.brake                     = info.brake;
    v.gas                       = info.gas;
    v.yaw_rate                  = info.yaw_rate;
    v.curvature                 = info.curvature;
    v.pos_conf                  = info.pos_confidence_m.or(v.pos_conf);
    v.hdg_conf                  = info.hdg_conf.or(v.hdg_conf);
    v.spd_conf                  = info.spd_conf.or(v.spd_conf);
    v.yaw_conf                  = info.yaw_conf.or(v.yaw_conf);
    v.v_len                     = info.v_len.or(v.v_len);
    v.v_wid                     = info.v_wid.or(v.v_wid);
    // always sync to latest frame_seq -> resync after overflow or long gaps
    v.frame_seq                 = info.frame_seq.or(v.frame_seq);
}

/// Returns `false` when both entries have known vehicle dimensions that differ
/// beyond `DIMENSION_TOLERANCE_M`. Dimensions are stable per trip.
fn dimensions_consistent(v: &VehicleEntry, info: &PacketInfo) -> bool {
    if let (Some(l1), Some(l2)) = (v.v_len, info.v_len) {
        if (l1 - l2).abs() > DIMENSION_TOLERANCE_M {
            return false;
        }
    }
    if let (Some(w1), Some(w2)) = (v.v_wid, info.v_wid) {
        if (w1 - w2).abs() > DIMENSION_TOLERANCE_M {
            return false;
        }
    }
    true
}

/// Returns `false` when both entries carry a known, non-zero curvature with
/// opposite signs. A left-turn vehicle cannot become a right-turn vehicle
/// within one pseudonym-change window. Zero curvature always passes.
fn curvature_sign_consistent(v: &VehicleEntry, info: &PacketInfo) -> bool {
    if let (Some(c1), Some(c2)) = (v.curvature, info.curvature) {
        let s1 = c1.signum();
        let s2 = c2.signum();
        if s1 != 0 && s2 != 0 && s1 != s2 {
            return false;
        }
    }
    true
}


/// Returns true if the 802.11 sequence number advanced by [1, max_delta] modulo 4096.
/// A forward delta of 0 means duplicate/replayed frame; > max_delta means the counter
/// already overshot the pseudonym-change window or wrapped in the wrong direction.
fn seq_advance_ok(old: u16, new: u16, max_delta: u16) -> bool {
    let delta = new.wrapping_sub(old) & (SEQ_MODULO - 1);
    delta >= 1 && delta <= max_delta
}

/// Predicts the vehicle position after `dt` seconds using trapezoidal
/// kinematic integration (constant yaw rate, constant longitudinal accel).
///
/// ETSI heading: 0 = North, 90 = East, clockwise.
/// Returns `None` when the last known position is unavailable.
fn predict_position(v: &VehicleEntry, dt: f64) -> Option<(f64, f64)> {
    let lat = v.lat?;
    let lon = v.lon?;
    let spd       = v.end_speed_kmh.unwrap_or(0.0) / 3.6;
    let hdg       = v.end_heading_deg.unwrap_or(0.0);
    let accel_ms2 = v.accel.unwrap_or(0.0);

    let dt = dt.min(MAX_PREDICTION_S);

    // curvature (0.0001 1/m per LSB, positive=left) contributes with weight 0.20
    // sign is negated to match the clockwise ETSI heading convention used below
    let curv_yaw = v.curvature.map(|c| -(spd * c as f64 * 1e-4).to_degrees());
    let yaw = match (v.yaw_rate, curv_yaw) {
        (Some(m), Some(c)) => 0.80 * m + 0.20 * c,
        (Some(m), None)    => m,
        (None,    Some(c)) => c,
        (None,    None)    => 0.0,
    };

    // trapezoidal rule: evaluate heading and speed at midpoint
    let hdg_mid = (hdg + yaw * dt * 0.5).to_radians();
    let spd_mid    = (spd + accel_ms2 * dt * 0.5).max(0.0);

    // sin -> East component, cos -> North component (ETSI clockwise convention)
    let dx_m = spd_mid * hdg_mid.sin() * dt;
    let dy_m = spd_mid * hdg_mid.cos() * dt;

    let cos_lat = lat.to_radians().cos().max(f64::EPSILON);
    Some((lat + dy_m / 111_111.0, lon + dx_m / (111_111.0 * cos_lat)))
}

/// Computes the half-width of the kinematic prediction corridor in metres
///
/// Sums four independent uncertainty contributions:
/// 1. Position measurement error (`pos_confidence_m`)
/// 2. Lateral displacement from heading uncertainty: `speed * sin(h_conf) * dt`
/// 3. Forward displacement from speed uncertainty: `speed_conf * dt`
/// 4. Heading drift from yaw-rate uncertainty: `0.5 * speed * yaw_conf * dt^2`
///
/// Clamped to `MIN_CORRIDOR_M` so stationary vehicles with accurate GPS
/// still have a usable link radius.
fn corridor_radius_m(v: &VehicleEntry, dt: f64) -> f64 {
    let dt       = dt.min(MAX_PREDICTION_S);
    let speed_ms = v.end_speed_kmh.unwrap_or(0.0) / 3.6;

    let pos_err = v.pos_conf.unwrap_or(DEFAULT_POS_CONFIDENCE_M);

    let h_conf = v
        .hdg_conf
        .unwrap_or(DEFAULT_HEADING_CONF_DEG)
        .to_radians();
    let h_delta = speed_ms * h_conf.sin() * dt;

    let spd_err = v
        .spd_conf
        .unwrap_or(DEFAULT_SPEED_CONF_MS)
        * dt;

    let yaw_conf = v
        .yaw_conf
        .unwrap_or(DEFAULT_YAW_CONF_DEG_S)
        .to_radians();
    let yaw_contrib = 0.5 * speed_ms * yaw_conf * dt * dt;

    (pos_err + h_delta + spd_err + yaw_contrib).max(MIN_CORRIDOR_M)
}
/// Weighted probabilistic link score
///
/// Weights: spatial 0.35, speed 0.275, heading 0.275, standstill 0.10 (sum 1.0).
/// Curvature contributes via `predict_position` (weight 0.20 blended into yaw).
/// Returns 0.0 on spatial hard reject (position outside 2x corridor).
fn score_link(v: &VehicleEntry, info: &PacketInfo, dt: f64) -> f64 {
    let spatial = match (v.lat, v.lon, info.lat, info.lon) {
        (Some(_), Some(_), Some(la2), Some(lo2)) => {
            // predict_position is Some because lat and lon are Some
            let (pred_lat, pred_lon) = predict_position(v, dt)
                .unwrap_or((v.lat.unwrap(), v.lon.unwrap()));
            let dist     = haversine_m(pred_lat, pred_lon, la2, lo2);
            let corridor = corridor_radius_m(v, dt);
            if dist > corridor * SPATIAL_HARD_REJECT_FACTOR {
                return 0.0;
            }
            (1.0 - dist / corridor).max(0.0)
        }
        _ => 0.5,
    };

    let speed = match (v.end_speed_kmh, info.speed_kmh) {
        (Some(s1), Some(s2)) => {
            let (ref_speed, tolerance) = if let Some(a) = v.accel {
                let predicted = (s1 + a * dt.min(MAX_ACCEL_EXTRAPOLATION_S) * 3.6).max(0.0);
                let complex = match (v.brake, v.gas) {
                    (Some(true), _) => a > TERRAIN_ACCEL_THRESHOLD_MS2,
                    (_, Some(true)) => a < -TERRAIN_ACCEL_THRESHOLD_MS2,
                    _ => false,
                };
                let tol = if complex {
                    MAX_SPEED_DELTA_KMH * TERRAIN_TOLERANCE_FACTOR
                } else {
                    MAX_SPEED_DELTA_KMH
                };
                (predicted, tol)
            } else {
                (s1, MAX_SPEED_DELTA_KMH)
            };
            (1.0 - (s2 - ref_speed).abs() / tolerance).max(0.0)
        }
        _ => 0.5,
    };

    let heading = match (v.end_heading_deg, info.heading_deg) {
        (Some(h1), Some(h2)) => {
            let d = (h1 - h2).abs() % 360.0;
            let delta = if d <= 180.0 { d } else { 360.0 - d };
            let max_d = (MAX_HEADING_DELTA_DEG_PER_S * dt.max(1.0)).min(90.0);
            (1.0 - delta / max_d).max(0.0)
        }
        _ => 0.5,
    };

    let standstill = if v.standstill >= MIN_STANDSTILL_S { 1.0 } else { 0.05 };

    let score = (0.350 * spatial + 0.275 * speed + 0.275 * heading + 0.100 * standstill)
        / (0.350 + 0.275 + 0.275 + 0.100);
    debug!(
        candidate = %v.last_mac, new_mac = %info.mac,
        spatial    = format_args!("{:.3}", spatial),
        speed      = format_args!("{:.3}", speed),
        heading    = format_args!("{:.3}", heading),
        standstill = format_args!("{:.3}", standstill),
        total      = format_args!("{:.3}", score),
        "lnik score"
    );
    score
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

