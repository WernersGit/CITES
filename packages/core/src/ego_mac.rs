use crate::{MacStats, RssiMeasurement};
use std::collections::HashMap;

pub struct EgoMac {
    /// Window size for rolling statistics
    pub window: usize,
    /// Minimum elements for valid rolling periods
    pub min_periods: usize,
    /// Measurements collected so far
    measurements: Vec<RssiMeasurement>,
    /// The chronologically sorted list of determined ego MAC addresses
    ego_mac_timeline: Vec<MacStats>,
}

impl EgoMac {
    pub fn new(window: usize, min_periods: usize) -> Self {
        Self {
            window,
            min_periods,
            measurements: Vec::new(),
            ego_mac_timeline: Vec::new(),
        }
    }

    pub fn insert_measurement(&mut self, timestamp_ms: i64, mac: String, rssi: f64) {
        self.measurements.push(RssiMeasurement {
            timestamp_ms,
            mac,
            rssi,
        });
    }

    pub fn insert_batch(&mut self, batch: impl IntoIterator<Item = RssiMeasurement>) {
        self.measurements.extend(batch);
    }

    /// scores and retursn the current ego MAC candidates
    pub fn evaluate(&mut self) -> &Vec<MacStats> {
        if self.measurements.is_empty() {
            return &self.ego_mac_timeline;
        }

        let mut stats = Self::scoring(&self.measurements, self.window, self.min_periods);
        
        // take top 50 candidates
        stats.truncate(50);


        if stats.is_empty() {
            self.ego_mac_timeline.clear();
            return &self.ego_mac_timeline;
        }

        // filter by rssi similarity to the best score candidate (index 0)
        let ref_rssi = stats[0].median_rssi;
        let candidates: Vec<MacStats> = stats
            .into_iter()
            .filter(|m| (m.median_rssi - ref_rssi).abs() <= 15.0)
            .collect();

        let mut timeline_macs: Vec<MacStats> = Vec::new();

        for candidate in candidates {
            let mut conflict = false;
            for accepted in &timeline_macs {
                let start = std::cmp::max(candidate.first_seen, accepted.first_seen);
                let end = std::cmp::min(candidate.last_seen, accepted.last_seen);
                if start < end {
                    let overlap_s = (end - start) as f64 / 1000.0;
                    if overlap_s > 0.1 {
                        conflict = true;
                        break;
                    }
                }
            }

            if !conflict {
                timeline_macs.push(candidate);
            }
        }

        // sort by first_seen chronologically
        timeline_macs.sort_by_key(|m| m.first_seen);
        
        self.ego_mac_timeline = timeline_macs;
        &self.ego_mac_timeline
    }

    pub fn get_timeline(&self) -> &[MacStats] {
        &self.ego_mac_timeline
    }

    /// Evaluates RSSI measurements and returns scored MAC statistics.
    /// Functions identically to the previous `build_statistics` implementation.
    pub fn scoring(measurements: &[RssiMeasurement], window: usize, min_periods: usize) -> Vec<MacStats> {
        if measurements.is_empty() {
            return Vec::new();
        }

        let mut grouped: HashMap<String, Vec<&RssiMeasurement>> = HashMap::new();
        for m in measurements {
            grouped.entry(m.mac.clone()).or_default().push(m);
        }

        let mut stats_list = Vec::new();

        for (mac, mut mac_meas) in grouped {
            // sort by timestamp
            mac_meas.sort_by_key(|m| m.timestamp_ms);

            let count = mac_meas.len();
            if count == 0 {
                continue;
            }

            let first_seen = mac_meas.first().unwrap().timestamp_ms;
            let last_seen = mac_meas.last().unwrap().timestamp_ms;

            let rssi_values: Vec<f64> = mac_meas.iter().map(|m| m.rssi).collect();

            let mean_rssi = rssi_values.iter().sum::<f64>() / count as f64;
            let median_rssi = Self::calculate_median(&mut rssi_values.clone());
            let std_rssi = Self::calculate_std(&rssi_values, mean_rssi);
            let iqr_rssi = Self::calculate_iqr(&mut rssi_values.clone());
            let mad_rssi = Self::calculate_mad(&rssi_values, median_rssi);

            let std_mean = Self::calculate_rolling_std_mean(&rssi_values, window, min_periods);

            let span_s = (last_seen - first_seen) as f64 / 1000.0;

            let score = Self::calculate_mac_score(
                count,
                median_rssi,
                std_rssi,
                iqr_rssi,
                std_mean,
                span_s,
            );

            stats_list.push(MacStats {
                mac,
                first_seen,
                last_seen,
                count,
                mean_rssi,
                median_rssi,
                std_rssi,
                iqr_rssi,
                mad_rssi,
                rolling_std_mean: std_mean,
                stability_score: score,
            });
        }

        // sort by score desc
        stats_list.sort_by(|a, b| {
            b.stability_score
                .partial_cmp(&a.stability_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        stats_list
    }

    fn calculate_mac_score(
        count: usize,
        median_rssi: f64,
        std_rssi: f64,
        iqr_rssi: f64,
        std_mean: f64,
        span_s: f64,
    ) -> f64 {
        let penalty = std_rssi.max(0.0) + iqr_rssi.max(0.0) + std_mean.max(0.0);
        let pen = penalty.ln_1p();

        let sample_score = (count as f64).sqrt();

        let mut base_score = sample_score / (1.0 + pen);

        let span_s = span_s.max(0.0);
        let span_bonus = span_s.sqrt();
        base_score *= 1.0 + span_bonus / 10.0;

        let clipped_rssi = median_rssi.clamp(-100.0, 0.0);
        let rssi_bonus = (100.0 + clipped_rssi).max(1.0);

        base_score * rssi_bonus * rssi_bonus
    }

    fn calculate_median(values: &mut [f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = values.len() / 2;
        if values.len() % 2 == 0 {
            (values[mid - 1] + values[mid]) / 2.0
        } else {
            values[mid]
        }
    }

    fn calculate_std(values: &[f64], mean: f64) -> f64 {
        if values.len() < 2 {
            return 0.0;
        }
        let variance: f64 = values.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
        variance.sqrt()
    }

    fn calculate_iqr(values: &mut [f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        
        let q25_idx = values.len() / 4;
        let q75_idx = (values.len() * 3) / 4;
        
        if values.len() > 1 {
            values[q75_idx] - values[q25_idx]
        } else {
            0.0
        }
    }

    fn calculate_mad(values: &[f64], median: f64) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let mut dev: Vec<f64> = values.iter().map(|&x| (x - median).abs()).collect();
        Self::calculate_median(&mut dev)
    }

    fn calculate_rolling_std_mean(values: &[f64], window: usize, min_periods: usize) -> f64 {
        if values.len() < min_periods || window < 2 || min_periods < 2 {
            return 0.0;
        }
        
        let mut stds = Vec::new();
        for i in 0..values.len() {
            let lo = if i + 1 > window { i + 1 - window } else { 0 };
            let hi = i + 1;
            let w_values = &values[lo..hi];
            if w_values.len() >= min_periods {
                let mean = w_values.iter().sum::<f64>() / w_values.len() as f64;
                let std = Self::calculate_std(w_values, mean);
                stds.push(std);
            }
        }
        
        if stds.is_empty() {
            0.0
        } else {
            stds.iter().sum::<f64>() / stds.len() as f64
        }
    }
}
