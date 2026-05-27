use std::collections::HashMap;
use crate::config::TrackingWarningConfig;

/// Reason a tracking warning was triggered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarnReason {
    /// Vehicle was continuously visible for at least `min_visible_minutes`
    Duration,
    /// Vehicle traveled at least `min_visible_km` without a large positional gap
    Distance,
}

struct VehicleWindow {
    first_seen_ms:  i64,
    last_seen_ms:   i64,
    last_lat:       Option<f64>,
    last_lon:       Option<f64>,
    accumulated_km: f64,
}

/// Stateful per-vehicle tracking warning checker
///
/// Call [`check_packet`] for every non-ego packet after the vehicle tracker has
/// assigned a virtual ID. The first threshold reached fires the warning and
/// resets the window so the vehicle can trigger again later.
pub struct TrackingWarningChecker {
    cfg:     TrackingWarningConfig,
    windows: HashMap<u32, VehicleWindow>,
}

impl TrackingWarningChecker {
    pub fn new(cfg: TrackingWarningConfig) -> Self {
        Self { cfg, windows: HashMap::new() }
    }

    /// Replace the active configuration (takes effect on the next packet)
    pub fn update_config(&mut self, cfg: TrackingWarningConfig) {
        self.cfg = cfg;
    }

    /// Drop state for a vehicle evicted by the vehicle tracker
    pub fn remove_vehicle(&mut self, vid: u32) {
        self.windows.remove(&vid);
    }

    /// Process one packet for a foreign vehicle
    ///
    /// Returns the first condition met, or `None`. The window is reset on a
    /// triggered warning or when a gap exceeds the configured tolerance.
    pub fn check_packet(
        &mut self,
        vid:          u32,
        timestamp_ms: i64,
        lat:          Option<f64>,
        lon:          Option<f64>,
    ) -> Option<WarnReason> {
        if !self.cfg.enabled {
            return None;
        }

        let gap_time_ms = self.cfg.gap_tolerance_secs as i64 * 1_000;
        let gap_dist_m  = self.cfg.gap_tolerance_km   * 1_000.0;
        let min_time_ms = self.cfg.min_visible_minutes as i64 * 60_000;
        let min_dist_km = self.cfg.min_visible_km;

        let window = self.windows.entry(vid).or_insert_with(|| VehicleWindow {
            first_seen_ms:  timestamp_ms,
            last_seen_ms:   timestamp_ms,
            last_lat:       lat,
            last_lon:       lon,
            accumulated_km: 0.0,
        });

        let dt_ms = timestamp_ms - window.last_seen_ms;

        // time gap exceeded: window reset -> not continuously tracked
        if dt_ms > gap_time_ms {
            *window = VehicleWindow {
                first_seen_ms:  timestamp_ms,
                last_seen_ms:   timestamp_ms,
                last_lat:       lat,
                last_lon:       lon,
                accumulated_km: 0.0,
            };
            return None;
        }

        // accumulate distance when positions are available
        if let (Some(lat2), Some(lon2)) = (lat, lon) {
            if let (Some(lat1), Some(lon1)) = (window.last_lat, window.last_lon) {
                let d_m = haversine_m(lat1, lon1, lat2, lon2);
                if d_m > gap_dist_m {
                    // positional gap too large: reset distance accumulator only
                    window.accumulated_km = 0.0;
                } else {
                    window.accumulated_km += d_m / 1_000.0;
                }
            }
        }

        window.last_seen_ms = timestamp_ms;
        window.last_lat     = lat;
        window.last_lon     = lon;

        // evaluate thresholds - duration first per user intent (urban vs extra-urban)
        let visible_ms = timestamp_ms - window.first_seen_ms;
        if visible_ms >= min_time_ms {
            window.first_seen_ms  = timestamp_ms;
            window.accumulated_km = 0.0;
            return Some(WarnReason::Duration);
        }
        if window.accumulated_km >= min_dist_km {
            window.first_seen_ms  = timestamp_ms;
            window.accumulated_km = 0.0;
            return Some(WarnReason::Distance);
        }

        None
    }
}

pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a  = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}
