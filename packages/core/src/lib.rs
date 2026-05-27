use serde::{Deserialize, Serialize};

pub mod config;
pub mod ble_protocol;
pub mod ble_handshake;
pub mod crc32;
pub mod ego_mac;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RssiMeasurement {
    pub timestamp_ms: i64,
    pub mac: String,
    pub rssi: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacStats {
    pub mac: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub count: usize,
    pub mean_rssi: f64,
    pub median_rssi: f64,
    pub std_rssi: f64,
    pub iqr_rssi: f64,
    pub mad_rssi: f64,
    pub rolling_std_mean: f64,
    pub stability_score: f64,
}
pub mod pcap_parser;
pub mod asn1;
pub mod vehicle_tracker;
pub mod tracking_warning;

pub mod c_its;
pub mod parser;

// pub mod asn1_decoder

/// Derives a u32 station-ID proxy from a colon-separated MAC string
///
/// Bytes at positions 2–5 (zero-indexed) of the 6-byte MAC are interpreted as a big-endian u32. 
/// This is a stable per-vehicle proxy for filtering before full ASN.1 StationID extraction is available.
pub fn mac_to_station_id(mac: &str) -> u32 {
    let bytes: Vec<u8> = mac
        .split(':')
        .filter_map(|s| u8::from_str_radix(s, 16).ok())
        .collect();
    if bytes.len() == 6 {
        u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]])
    } else {
        0
    }
}
