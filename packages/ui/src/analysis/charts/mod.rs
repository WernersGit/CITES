mod vehicle_table;
mod ipg_chart;
mod mac_timeline;
pub use mac_timeline::MacTimeline;
mod speed_chart;
mod trajectory_chart;
mod clock_drift_chart;
mod cam_delta_chart;
mod vehicle_length_chart;
mod accel_control_chart;
mod map_chart;

pub use vehicle_table::VehicleTable;
pub use ipg_chart::IpgBoxPlot;
pub use speed_chart::SpeedProfileChart;
pub use clock_drift_chart::ClockDriftChart;
pub use cam_delta_chart::CamDeltaChart;
pub use vehicle_length_chart::VehicleLengthChart;
pub use accel_control_chart::AccelControlChart;
pub use map_chart::MapChart;

use dioxus::prelude::*;
use core_logic::pcap_parser::ParsedPacket;
use core_logic::parser::decoder::ItsPayload;
use core_logic::vehicle_tracker::{VehicleTracker, PacketInfo, LAT_LON_SCALE, SPEED_SCALE, HEADING_SCALE};
use core_logic::ego_mac::EgoMac;
use platform::stats::{MacTimelinePoint, derive_timeline_points};
use chrono::{TimeZone, Utc};
use std::collections::HashMap;
use crate::trajectory::{MacTrajectory, build_trajectories};

pub const COLORS: &[&str] = &[
    "#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd",
    "#8c564b", "#e377c2", "#7f7f7f", "#bcbd22", "#17becf",
    "#aec7e8", "#ffbb78", "#98df8a", "#ff9896", "#c5b0d5",
    "#c49c94", "#f7b6d2", "#c7c7c7", "#dbdb8d", "#9edae5",
];

// shared data types

#[derive(Clone, PartialEq)]
pub struct VehicleRow {
    pub virtual_id: u32,
    pub macs: Vec<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub packet_count: u64,
}

#[derive(Clone, PartialEq)]
pub struct IpgStats {
    pub mac: String,
    pub q1_ms: f64,
    pub median_ms: f64,
    pub q3_ms: f64,
    pub whisker_lo_ms: f64,
    pub whisker_hi_ms: f64,
    pub packet_count: u64,
}

#[derive(Clone, PartialEq)]
pub struct SpeedSample {
    pub timestamp_ms: i64,
    pub mac: String,
    pub speed_kmh: f64,
}


#[derive(Clone, PartialEq)]
pub struct ClockDriftSample {
    pub mac: String,
    pub capture_ms: i64,
    pub gen_delta_ms: u32,
}

#[derive(Clone, PartialEq)]
pub struct VehicleLengthSample {
    pub mac: String,
    pub timestamp_ms: i64,
    pub length_m: f64,
}

#[derive(Clone, PartialEq)]
pub struct AccelSample {
    pub mac: String,
    pub timestamp_ms: i64,
    pub accel: i64,
}

/// Pre-computed acceleration control flag event for the timeline chart.
#[derive(Clone, PartialEq)]
pub struct AccelControlEvent {
    pub mac: String,
    pub timestamp_ms: i64,
    /// 7-bit bitmask: bit7=Brakes, bit6=Gas, bit5=EmgBrake, bit4=CollWarn, bit3=ACC, bit2=Cruise, bit1=SpeedLim
    pub flags: u8,
}

#[derive(Clone, PartialEq)]
pub struct DecodeStats {
    pub total: u64,
    pub with_gnw: u64,
    pub with_btp: u64,
    pub with_payload: u64,
    pub as_cam: u64,
    pub as_denm: u64,
    pub as_unsupported: u64,
}

#[derive(Clone, PartialEq)]
pub struct AnalysisData {
    pub timeline: Vec<MacTimelinePoint>,
    pub vehicles: Vec<VehicleRow>,
    pub ipg_stats: Vec<IpgStats>,
    pub speed_series: Vec<SpeedSample>,
    pub trajectory: Vec<MacTrajectory>,
    pub clock_drift: Vec<ClockDriftSample>,
    pub vehicle_lengths: Vec<VehicleLengthSample>,
    pub accel_series: Vec<AccelSample>,
    pub accel_control_events: Vec<AccelControlEvent>,
    pub mac_order: Vec<String>,
    pub min_ms: i64,
    pub max_ms: i64,
    pub total_packets: u64,
    pub decode_stats: DecodeStats,
}

// computation

pub fn compute_analysis(packets: &[ParsedPacket]) -> AnalysisData {
    let empty = DecodeStats { total: 0, with_gnw: 0, with_btp: 0, with_payload: 0, as_cam: 0, as_denm: 0, as_unsupported: 0 };
    if packets.is_empty() {
        return AnalysisData {
            timeline: vec![], vehicles: vec![], ipg_stats: vec![],
            speed_series: vec![], trajectory: vec![], clock_drift: vec![],
            vehicle_lengths: vec![], accel_series: vec![], accel_control_events: vec![],
            mac_order: vec![], min_ms: 0, max_ms: 0, total_packets: 0,
            decode_stats: empty,
        };
    }

    let stats = {
        use core_logic::parser::decoder::ItsPayload;
        let mut s = DecodeStats { total: packets.len() as u64, with_gnw: 0, with_btp: 0, with_payload: 0, as_cam: 0, as_denm: 0, as_unsupported: 0 };
        for pkt in packets {
            if pkt.gnw_info.is_some() { s.with_gnw += 1; }
            if pkt.btp_b_info.is_some() { s.with_btp += 1; }
            match &pkt.payload {
                Some(ItsPayload::Cam(_))        => { s.with_payload += 1; s.as_cam += 1; }
                Some(ItsPayload::Denm(_))       => { s.with_payload += 1; s.as_denm += 1; }
                Some(ItsPayload::Unsupported)   => { s.with_payload += 1; s.as_unsupported += 1; }
                None => {}
            }
        }
        s
    };

    let timeline = derive_timeline_points(packets);

    let mut mac_ts: HashMap<String, Vec<i64>> = HashMap::new();
    for pkt in packets {
        mac_ts.entry(pkt.mac.clone()).or_default().push(pkt.timestamp_ms);
    }

    let mut mac_order: Vec<String> = mac_ts.keys().cloned().collect();
    mac_order.sort_by(|a, b| mac_ts[b].len().cmp(&mac_ts[a].len()));

    // ego MAC first — cheaper than full tracker pass
    let mut ego_calc = EgoMac::new(10_000, 5);
    for pkt in packets {
        ego_calc.insert_measurement(pkt.timestamp_ms, pkt.mac.clone(), pkt.rssi);
    }
    let ego_tl = ego_calc.evaluate().to_vec();
    let ego_macs: Vec<String> = ego_tl.iter().map(|s| s.mac.clone()).collect();

    let mut tracker = VehicleTracker::new();
    tracker.set_ego_macs(ego_macs.iter().cloned());
    for pkt in packets {
        // GNW LPV speed: 15-bit, 0.01 m/s units; 0x7FFF (32767) = unavailable
        // prefer cam.speed_kmh when the packet carries a decoded CAM payload
        let speed_kmh = if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            cam.speed_kmh
        } else {
            pkt.gnw_info.as_ref().and_then(|g| {
                if g.speed >= 0x7FFF { None } else { Some(g.speed as f64 * SPEED_SCALE) }
            })
        };
        let heading_deg = pkt.gnw_info.as_ref().and_then(|g| {
            if g.heading >= 3601 { None } else { Some(g.heading as f64 * HEADING_SCALE) }
        });
        let cam = pkt.payload.as_ref().and_then(|p| {
            if let ItsPayload::Cam(cam) = p { Some(cam.as_ref()) } else { None }
        });
        tracker.insert_packet(PacketInfo {
            mac: pkt.mac.clone(),
            timestamp_ms: pkt.timestamp_ms,
            lat: pkt.gnw_info.as_ref().map(|g| g.latitude as f64 * LAT_LON_SCALE),
            lon: pkt.gnw_info.as_ref().map(|g| g.longitude as f64 * LAT_LON_SCALE),
            pos_confidence_m:  cam.and_then(|c| c.pos_confidence_m),
            speed_kmh,
            spd_conf:          cam.and_then(|c| c.speed_confidence_ms),
            heading_deg,
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

    let mut vehicles: Vec<VehicleRow> = tracker.iter_vehicles().map(|(vid, macs)| {
        let (start_ms, end_ms, pkt_count) = macs.iter().fold(
            (i64::MAX, i64::MIN, 0u64),
            |(s, e, n), mac| match mac_ts.get(mac) {
                Some(ts) => (
                    s.min(*ts.iter().min().unwrap_or(&0)),
                    e.max(*ts.iter().max().unwrap_or(&0)),
                    n + ts.len() as u64,
                ),
                None => (s, e, n),
            },
        );
        VehicleRow {
            virtual_id: vid,
            macs: macs.to_vec(),
            start_ms: if start_ms == i64::MAX { 0 } else { start_ms },
            end_ms: if end_ms == i64::MIN { 0 } else { end_ms },
            packet_count: pkt_count,
        }
    }).collect();

    // add ego vehicle as virtualID=0 if ego MACs were detected
    if !ego_macs.is_empty() {
        let (ego_start, ego_end, ego_count) = ego_macs.iter().fold(
            (i64::MAX, i64::MIN, 0u64),
            |(s, e, n), mac| match mac_ts.get(mac) {
                Some(ts) => (
                    s.min(*ts.iter().min().unwrap_or(&0)),
                    e.max(*ts.iter().max().unwrap_or(&0)),
                    n + ts.len() as u64,
                ),
                None => (s, e, n),
            },
        );
        vehicles.push(VehicleRow {
            virtual_id: 0,
            macs: ego_macs,
            start_ms: if ego_start == i64::MAX { 0 } else { ego_start },
            end_ms: if ego_end == i64::MIN { 0 } else { ego_end },
            packet_count: ego_count,
        });
    }

    let ipg_stats: Vec<IpgStats> = mac_order.iter().take(10).filter_map(|mac| {
        let ts = mac_ts.get(mac)?;
        if ts.len() < 4 { return None; }
        let mut sorted = ts.clone();
        sorted.sort();
        let mut gaps: Vec<f64> = sorted.windows(2)
            .map(|w| (w[1] - w[0]) as f64)
            .filter(|&g| g > 0.0 && g < 2000.0)
            .collect();
        if gaps.len() < 3 { return None; }
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q1 = percentile(&gaps, 25.0);
        let q3 = percentile(&gaps, 75.0);
        let iqr = q3 - q1;
        let wlo = gaps.iter().cloned().filter(|&v| v >= q1 - 1.5 * iqr).fold(f64::INFINITY, f64::min);
        let whi = gaps.iter().cloned().filter(|&v| v <= q3 + 1.5 * iqr).fold(f64::NEG_INFINITY, f64::max);
        Some(IpgStats {
            mac: mac.clone(),
            q1_ms: q1,
            median_ms: percentile(&gaps, 50.0),
            q3_ms: q3,
            whisker_lo_ms: if wlo.is_finite() { wlo } else { q1 },
            whisker_hi_ms: if whi.is_finite() { whi } else { q3 },
            packet_count: ts.len() as u64,
        })
    }).collect();

    // use cam.speed_kmh (from cam HighFrequencyContainer, already converted in the parser)
    // instead of gnw_info.speed (gnw long position vector via static byte offsets).
    // The cam field matches python CamAnalyzer.py's its.speedValue / 100 * 3.6
    // and is reliable even for secured packets where gnw offsets can be wrong
    let spd: Vec<SpeedSample> = packets.iter().filter_map(|pkt| {
        if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            let speed_kmh = cam.speed_kmh?;
            Some(SpeedSample {
                timestamp_ms: pkt.timestamp_ms,
                mac: pkt.mac.clone(),
                speed_kmh,
            })
        } else { None }
    }).collect();

    let trajectory = build_trajectories(packets, None);

    let clock_drift: Vec<ClockDriftSample> = packets.iter().filter_map(|pkt| {
        if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            Some(ClockDriftSample {
                mac: pkt.mac.clone(),
                capture_ms: pkt.timestamp_ms,
                gen_delta_ms: cam.gen_delta_time_ms,
            })
        } else { None }
    }).collect();

    let lengths: Vec<VehicleLengthSample> = packets.iter().filter_map(|pkt| {
        if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            let len_m = cam.vehicle_length_m?;
            Some(VehicleLengthSample {
                mac: pkt.mac.clone(),
                timestamp_ms: pkt.timestamp_ms,
                length_m: len_m,
            })
        } else { None }
    }).collect();

    let accel: Vec<AccelSample> = packets.iter().filter_map(|pkt| {
        if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            let accel_ms2 = cam.longitudinal_accel?;
            Some(AccelSample {
                mac: pkt.mac.clone(),
                timestamp_ms: pkt.timestamp_ms,
                accel: (accel_ms2 * 10.0).round() as i64,
            })
        } else { None }
    }).collect();

    let accel_control_events: Vec<AccelControlEvent> = packets.iter().filter_map(|pkt| {
        if let Some(ItsPayload::Cam(cam)) = &pkt.payload {
            let ac = cam.accel_control.as_ref()?;
            Some(AccelControlEvent {
                mac: pkt.mac.clone(),
                timestamp_ms: pkt.timestamp_ms,
                flags: ac.to_byte(),
            })
        } else { None }
    }).collect();

    let min_ms = packets.iter().map(|p| p.timestamp_ms).min().unwrap_or(0);
    let max_ms = packets.iter().map(|p| p.timestamp_ms).max().unwrap_or(0);

    AnalysisData {
        timeline, vehicles, ipg_stats, speed_series: spd, trajectory,
        clock_drift, vehicle_lengths: lengths, accel_series: accel, accel_control_events,
        mac_order, min_ms, max_ms, total_packets: packets.len() as u64,
        decode_stats: stats,
    }
}

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 { return 0.0; }
    let idx = p / 100.0 * (n - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    sorted[lo] * (1.0 - idx.fract()) + sorted[hi] * idx.fract()
}

pub fn fmt_hms(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms).unwrap().format("%H:%M:%S").to_string()
}

// ChartCard

#[derive(Props, Clone, PartialEq)]
pub struct ChartCardProps {
    pub title: String,
    pub children: Element,
}

#[component]
pub fn ChartCard(props: ChartCardProps) -> Element {
    rsx! {
        div { style: "padding: 12px 14px; border: 1px solid #dee2e6; border-radius: 8px; background: #fff;",
            h3 { style: "margin: 0 0 10px; font-size: 0.92rem; font-weight: 600; color: #212529;",
                "{props.title}"
            }
            {props.children}
        }
    }
}

// PlaceholderChart

#[derive(Props, Clone, PartialEq)]
pub struct PlaceholderProps {
    pub title: String,
    pub note: String,
}

#[component]
pub fn PlaceholderChart(props: PlaceholderProps) -> Element {
    rsx! {
        ChartCard { title: props.title.clone(),
            div { style: "display: flex; flex-direction: column; align-items: center; justify-content: center; \
                        min-height: 120px; background: #f8f9fa; border-radius: 6px; gap: 6px; padding: 1rem;",
                span { style: "font-size: 1.4rem;", "🔬" }
                span { style: "font-size: 0.8rem; color: #888; text-align: center; max-width: 280px; line-height: 1.4;",
                    "{props.note}"
                }
            }
        }
    }
}
