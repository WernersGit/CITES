/// Real-time state of a tracked vehicle, updated by the live reassembly loop.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveVehicleState {
    pub virtual_id: u32,
    pub macs: Vec<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub speed_kmh: Option<f64>,
    pub heading_deg: Option<f64>,
    pub is_ego: bool,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PcapStats {
    pub total_packets: u64,
    pub total_bytes: u64,
    /// BLE reassmbly failures (incomplete multi-fragment messages).
    pub dropped_fragments: u64,
    /// Gaps detected in the PCAP sequence number stream after reassembly.
    /// Each unit represents one missing archive packet.
    pub missed_packets: u64,
    pub is_incomplete: bool,
}

/// A single sampled presence point for one MAC address, used by the timeline chart.
/// Stored at 1-second resolution: at most one point per MAC per second.
#[derive(Debug, Clone, PartialEq)]
pub struct MacTimelinePoint {
    pub timestamp_ms: i64,
    pub mac: String,
}

/// Derives timeline presence points from a batch of already-parsed packets.
///
/// Applies the same 1-second bucket deduplication used in the live reassembly
/// loop, making offline-loaded data visually consistent with live data.
pub fn derive_timeline_points(
    packets: &[core_logic::pcap_parser::ParsedPacket],
) -> Vec<MacTimelinePoint> {
    use std::collections::HashMap;
    const BUCKET_MS: i64 = 1_000;
    let mut last_bucket: HashMap<String, i64> = HashMap::new();
    let mut points = Vec::new();
    for pkt in packets {
        let bucket = pkt.timestamp_ms / BUCKET_MS;
        if last_bucket.get(&pkt.mac).copied().unwrap_or(-1) != bucket {
            last_bucket.insert(pkt.mac.clone(), bucket);
            points.push(MacTimelinePoint {
                timestamp_ms: bucket * BUCKET_MS,
                mac: pkt.mac.clone(),
            });
        }
    }
    points
}
