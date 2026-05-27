use serde::{Deserialize, Serialize};
use sysinfo::System;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage: f32,
    pub ram_used_mb: f64,
    pub ram_total_mb: f64,
    pub temp_celsius: f32,
    pub mac_address: String,
}

impl SystemMetrics {
    /// Serializes metrics into a compact comma-separated string to save bandwith over BLE/IP.
    pub fn to_csv(&self) -> String {
        format!("{:.1},{:.1},{:.1},{:.1},{}", 
            self.cpu_usage, 
            self.ram_used_mb, 
            self.ram_total_mb, 
            self.temp_celsius, 
            self.mac_address)
    }

    pub fn from_csv(csv: &str) -> Result<Self, String> {
        let clean_csv = csv.trim().trim_matches('\0');
        let parts: Vec<&str> = clean_csv.split(',').collect();
        
        if parts.len() < 5 {
            return Err(format!("bad CSV: expected 5 fields, got {}: '{}'", parts.len(), clean_csv));
        }

        Ok(Self {
            cpu_usage: parts[0].parse().unwrap_or(0.0),
            ram_used_mb: parts[1].parse().unwrap_or(0.0),
            ram_total_mb: parts[2].parse().unwrap_or(0.0),
            temp_celsius: parts[3].parse().unwrap_or(0.0),
            // Rejoin remaining parts in case the MAC address itself contained commas? Unlikely.
            mac_address: parts[4].to_string(),
        })
    }
}

/// Unified node status: system metrics + replay counter.
///
/// Served as JSON over `GET /status` (HTTP) and as CSV over
/// `METRICS_CHARACTERISTIC` READ (BLE).  The CSV layout is:
/// `cpu,ram_used,ram_total,temp,replay_count` — five fields, no MAC address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub cpu_usage: f32,
    pub ram_used_mb: f64,
    pub ram_total_mb: f64,
    pub temp_celsius: f32,
    pub replay_count: u64,
}

impl NodeStatus {
    pub fn to_csv(&self) -> String {
        format!(
            "{:.1},{:.1},{:.1},{:.1},{}",
            self.cpu_usage, self.ram_used_mb, self.ram_total_mb,
            self.temp_celsius, self.replay_count
        )
    }

    pub fn from_csv(csv: &str) -> Result<Self, String> {
        let clean = csv.trim().trim_matches('\0');
        let parts: Vec<&str> = clean.split(',').collect();
        if parts.len() < 5 {
            return Err(format!("expected 5 fields, got {}: '{}'", parts.len(), clean));
        }
        Ok(Self {
            cpu_usage:    parts[0].parse().unwrap_or(0.0),
            ram_used_mb:  parts[1].parse().unwrap_or(0.0),
            ram_total_mb: parts[2].parse().unwrap_or(0.0),
            temp_celsius: parts[3].parse().unwrap_or(0.0),
            replay_count: parts[4].parse().unwrap_or(0),
        })
    }
}

/// On-demand tracking report served by a Cloud node at `GET /tracking/report`.
///
/// A Local node never produces this.  The client requests it explicitly and may
/// use the result to cross-check or supplement its own local tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackingReport {
    /// Best ego-MAC candidate, or `None` when insufficient data.
    pub ego_mac: Option<String>,
    /// Stability score of the ego MAC candidate.
    pub ego_stability_score: f64,
    /// All virtual vehicles seen (ego included as virtual_id = 0).
    pub vehicles: Vec<VirtualVehicle>,
    /// Number of packets analysed to produce this report.
    pub packets_analysed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualVehicle {
    /// 0 = ego, 1+ = foreign.
    pub virtual_id: u32,
    /// Ordered pseudonym chain, most recent last.
    pub macs: Vec<String>,
}

pub struct MetricsService {
    sys: System,
}

impl MetricsService {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Self { sys }
    }

    pub fn gather_metrics(&mut self) -> SystemMetrics {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        // CPU usage (average over all cores)
        let cpus = self.sys.cpus();
        let cpu = if !cpus.is_empty() {
            let total: f32 = cpus.iter().map(|cpu| cpu.cpu_usage()).sum();
            total / cpus.len() as f32
        } else {
            0.0
        };

        // RAM usage
        let total_mb = self.sys.total_memory() as f64 / 1024.0 / 1024.0;
        let used_mb = self.sys.used_memory() as f64 / 1024.0 / 1024.0;

        // pi temperature, 0 elsewhere
        let temp = Self::read_pi_temperature();
        // mac address (dummy for now)
        let mac_address = "FF:FF:FF:FF:FF:FF".to_string();

        SystemMetrics {
            cpu_usage: cpu,
            ram_used_mb: used_mb,
            ram_total_mb: total_mb,
            temp_celsius: temp,
            mac_address,
        }
    }

    /// Read Raspberry Pi CPU temperature directly from sysfs
    fn read_pi_temperature() -> f32 {
        if let Ok(content) = std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp") {
            if let Ok(milli_celsius) = content.trim().parse::<f32>() {
                return milli_celsius / 1000.0;
            }
        }
        0.0 // fallback if not on a Pi
    }
}
