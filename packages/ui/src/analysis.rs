mod charts;

use crate::source_picker::SourcePicker;
use charts::{
    compute_analysis, AnalysisData, VehicleRow,
    ChartCard,
    VehicleTable, IpgBoxPlot, SpeedProfileChart,
    ClockDriftChart, CamDeltaChart, VehicleLengthChart, AccelControlChart,
    MapChart, MacTimeline,
};

use dioxus::prelude::*;
use core_logic::pcap_parser::ParsedPacket;
use core_logic::parser::decoder::ItsPayload;
use platform::ConnectionService;

const ANALYSIS_CSS: Asset = asset!("/assets/styling/analysis.css");

#[derive(Clone, PartialEq)]
enum AnalysisMode {
    Online,
    Offline,
}

// filter state

#[derive(Clone, PartialEq, Default)]
struct FilterState {
    /// Seconds offset from first packet timestamp (inclusive lower bound).
    t_start_s: String,
    /// Seconds offset from first packet timestamp (inclusive upper bound).
    t_end_s: String,
    /// Partial MAC address substring match (case-insensitive).
    mac: String,
    /// Target vehicle length in metres (filters to packets within tolerance).
    veh_len: String,
    /// Tolerance in metres for vehicle length filter (default: 0.5).
    veh_len_tol: String,
    /// Restrict to MACs belonging to this virtual vehicle ID.
    vehicle_id: Option<u32>,
}

/// Apply time, MAC, and vehicle-length fitlers (everything except vehicle_id)
fn apply_base_filters(pkts: &[ParsedPacket], fs: &FilterState) -> Vec<ParsedPacket> {
    let min_ts = pkts.iter().map(|p| p.timestamp_ms).min().unwrap_or(0);

    let t_start: Option<i64> = fs.t_start_s.trim().parse::<f64>().ok()
        .map(|s| min_ts + (s * 1000.0) as i64);
    let t_end: Option<i64> = fs.t_end_s.trim().parse::<f64>().ok()
        .map(|s| min_ts + (s * 1000.0) as i64);
    let mac_filter = fs.mac.trim().to_lowercase();
    let veh_len: Option<f64> = fs.veh_len.trim().parse().ok();
    let veh_len_tol: f64 = fs.veh_len_tol.trim().parse().unwrap_or(0.5);

    pkts.iter().filter(|p| {
        if let Some(t) = t_start { if p.timestamp_ms < t { return false; } }
        if let Some(t) = t_end   { if p.timestamp_ms > t { return false; } }
        if !mac_filter.is_empty() && !p.mac.to_lowercase().contains(&*mac_filter) { return false; }
        if let Some(target) = veh_len {
            match &p.payload {
                Some(ItsPayload::Cam(cam)) => {
                    match cam.vehicle_length_m {
                        Some(len) if (len - target).abs() <= veh_len_tol => {}
                        _ => return false,
                    }
                }
                _ => return false,
            }
        }
        true
    }).cloned().collect()
}

// main component

#[component]
pub fn Analysis() -> Element {
    let mut mode         = use_signal(|| AnalysisMode::Online);
    let mut show_picker  = use_signal(|| false);
    let mut pkts: Signal<Vec<ParsedPacket>> = use_signal(Vec::new);
    let mut filter       = use_signal(FilterState::default);
    let mut filters_open = use_signal(|| false);

    let connection = use_context::<ConnectionService>();

    // TODO: cache pass 1 result to avoid full recompute on vehicle filter change
    // two-pass analyis: pass 1 gets all vehicles, pass 2 restricts by vehicle_id
    let result: Memo<Option<(AnalysisData, Vec<VehicleRow>)>> = use_memo(move || {
        let pkt_data = pkts.read();
        if pkt_data.is_empty() { return None; }
        let fs = filter.read();

        let filtered = apply_base_filters(&pkt_data, &fs);
        if filtered.is_empty() { return None; }

        let base_data = compute_analysis(&filtered);
        let vehicles = base_data.vehicles.clone();

        if let Some(vid) = fs.vehicle_id {
            let macs: Vec<String> = vehicles.iter()
                .find(|v| v.virtual_id == vid)
                .map(|v| v.macs.clone())
                .unwrap_or_default();

            if !macs.is_empty() {
                let mac_pkts: Vec<ParsedPacket> = filtered.into_iter()
                    .filter(|p| macs.contains(&p.mac))
                    .collect();

                if !mac_pkts.is_empty() {
                    return Some((compute_analysis(&mac_pkts), vehicles));
                }
            }
        }

        Some((base_data, vehicles))
    });

    let raw_duration_s: Memo<Option<u64>> = use_memo(move || {
        let pkts = pkts.read();
        if pkts.is_empty() { return None; }
        let t0 = pkts.iter().map(|p| p.timestamp_ms).min().unwrap_or(0);
        let t1 = pkts.iter().map(|p| p.timestamp_ms).max().unwrap_or(0);
        Some(((t1 - t0) / 1000).max(0) as u64)
    });

    let online_active = *mode.read() == AnalysisMode::Online;

    rsx! {
        document::Link { rel: "stylesheet", href: ANALYSIS_CSS }

        div { class: "analysis-page",
            h1 { class: "page-title", "Analysis" }

            if online_active {
                div { class: "online-panel",
                    h2 { class: "online-panel-title", "MAC Address Presence Timeline" }
                    p { class: "online-panel-desc",
                        "Live data from the node. Each dot marks a second with at least one packet from that MAC."
                    }
                    MacTimeline { points: connection.mac_timeline.read().clone() }
                }
            } else {
                match result.read().clone() {
                    None => rsx! {
                        div { class: "empty-state",
                            span { class: "empty-state-text", "No data loaded." }
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| *show_picker.write() = true,
                                "Select source"
                            }
                        }
                    },
                    Some((data, all_vehicles)) => {
                        let d = data.clone();
                        rsx! {
                            // filter panel
                            div { class: "filter-wrapper",
                                button {
                                    class: "filter-toggle",
                                    onclick: move |_| {
                                        let v = *filters_open.read();
                                        *filters_open.write() = !v;
                                    },
                                    if *filters_open.read() {
                                        "Hide Filters"
                                    } else {
                                        "Show Filters"
                                    }
                                }
                                if *filters_open.read() {
                                    FilterPanel {
                                        filter: filter.read().clone(),
                                        duration_s: raw_duration_s.read().clone(),
                                        all_vehicles: all_vehicles.clone(),
                                        on_change: move |fs: FilterState| {
                                            *filter.write() = fs;
                                        },
                                        on_clear: move |_| {
                                            *filter.write() = FilterState::default();
                                        },
                                    }
                                }
                            }

                            // summary bar
                            div { class: "summary-bar",
                                stat_pill { label: "Packets", value: "{d.total_packets}" }
                                stat_pill { label: "MACs", value: "{d.mac_order.len()}" }
                                stat_pill { label: "Vehicles", value: "{d.vehicles.len()}" }
                                stat_pill { label: "Duration", value: "{fmt_duration(d.min_ms, d.max_ms)}" }
                            }

                            // decode pipeline stats
                            {
                                let s = &d.decode_stats;
                                rsx! {
                                    div { class: "decode-stats",
                                        span { "Decode pipeline:" }
                                        span { "Total: {s.total}" }
                                        span { "GNW: {s.with_gnw}" }
                                        span { "BTP-B: {s.with_btp}" }
                                        span { "Payload decoded: {s.with_payload}" }
                                        span { style: "color: var(--accent-green); font-weight: 600;", "CAM: {s.as_cam}" }
                                        span { style: "color: var(--accent-red);", "DENM: {s.as_denm}" }
                                        span { style: "color: var(--text-muted);", "Unsupported: {s.as_unsupported}" }
                                    }
                                }
                            }

                            // virtual vehicle table (full width)
                            div { class: "chart-grid-full",
                                ChartCard { title: "Virtual Vehicle Mapping Table",
                                    VehicleTable { vehicles: d.vehicles.clone() }
                                }
                            }

                            // chart grid
                            div { class: "chart-grid",
                                div { class: "chart-grid-full",
                                    ChartCard { title: "MAC Address Presence Timeline",
                                        p { style: "margin: 0 0 0.6rem; font-size: 0.82rem; color: var(--text-muted);",
                                            "One row per MAC address. Each dot marks a 1-second bucket with at least one received packet."
                                        }
                                        MacTimeline { points: d.timeline.clone() }
                                    }
                                }

                                ChartCard { title: "Inter-Packet Gap Distribution (top 10 MACs)",
                                    IpgBoxPlot { stats: d.ipg_stats.clone() }
                                }

                                ChartCard { title: "Clock Drift Tracking",
                                    ClockDriftChart {
                                        samples: d.clock_drift.clone(),
                                        mac_order: d.mac_order.clone(),
                                        min_ms: d.min_ms,
                                        max_ms: d.max_ms,
                                    }
                                }

                                ChartCard { title: "CAM Generation Delta Time (Sawtooth)",
                                    CamDeltaChart {
                                        samples: d.clock_drift.clone(),
                                        mac_order: d.mac_order.clone(),
                                        min_ms: d.min_ms,
                                        max_ms: d.max_ms,
                                    }
                                }

                                ChartCard { title: "Speed Profile over Time",
                                    SpeedProfileChart {
                                        series: d.speed_series.clone(),
                                        mac_order: d.mac_order.clone(),
                                        min_ms: d.min_ms,
                                        max_ms: d.max_ms,
                                    }
                                }

                                div { class: "chart-grid-full",
                                    ChartCard { title: "Spatial Trajectory (Interactive Map)",
                                        MapChart {
                                            trajectories: d.trajectory.clone(),
                                            vehicles: d.vehicles.clone(),
                                        }
                                    }
                                }

                                ChartCard { title: "Vehicle Length Profile",
                                    VehicleLengthChart {
                                        samples: d.vehicle_lengths.clone(),
                                        mac_order: d.mac_order.clone(),
                                        min_ms: d.min_ms,
                                        max_ms: d.max_ms,
                                    }
                                }

                                div { class: "chart-grid-full",
                                    ChartCard { title: "Acceleration Control Flags Timeline",
                                        AccelControlChart {
                                            events: d.accel_control_events.clone(),
                                            mac_order: d.mac_order.clone(),
                                            min_ms: d.min_ms,
                                            max_ms: d.max_ms,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // mode toggle bar (fixed at bottom)
        div { class: "mode-bar",
            button {
                class: if online_active { "mode-btn active" } else { "mode-btn" },
                onclick: move |_| *mode.write() = AnalysisMode::Online,
                "Online"
            }
            button {
                class: if !online_active { "mode-btn active" } else { "mode-btn" },
                onclick: move |_| {
                    *mode.write() = AnalysisMode::Offline;
                    *show_picker.write() = true;
                },
                "Offline"
            }
        }

        if *show_picker.read() {
            SourcePicker {
                on_dismiss: move |_| {
                    *show_picker.write() = false;
                    if pkts.read().is_empty() {
                        *mode.write() = AnalysisMode::Online;
                    }
                },
                on_loaded: move |loaded: Vec<ParsedPacket>| {
                    *pkts.write() = loaded;
                    *filter.write() = FilterState::default();
                    *show_picker.write() = false;
                },
            }
        }
    }
}

// filter panel

#[derive(Props, Clone, PartialEq)]
struct FilterPanelProps {
    filter: FilterState,
    duration_s: Option<u64>,
    all_vehicles: Vec<VehicleRow>,
    on_change: EventHandler<FilterState>,
    on_clear: EventHandler<()>,
}

#[component]
fn FilterPanel(props: FilterPanelProps) -> Element {
    let dur_hint = props.duration_s.map(|s| format!("0 - {s}")).unwrap_or_default();

    let mut veh_opts: Vec<(u32, String)> = props.all_vehicles.iter().map(|v| {
        let label = if v.virtual_id == 0 {
            format!("ID 0 - Ego ({} MACs, {} pkts)", v.macs.len(), v.packet_count)
        } else {
            format!("ID {} - {} MACs, {} pkts", v.virtual_id, v.macs.len(), v.packet_count)
        };
        (v.virtual_id, label)
    }).collect();
    veh_opts.sort_by_key(|(id, _)| *id);

    rsx! {
        div { class: "filter-panel",

            // time range
            div { class: "filter-group",
                span { class: "filter-label", "Time range (seconds from start)" }
                span { class: "filter-hint", "Capture: {dur_hint} s" }
                div { class: "filter-inline",
                    input {
                        r#type: "number",
                        placeholder: "Start",
                        value: "{props.filter.t_start_s}",
                        min: "0",
                        class: "filter-input",
                        oninput: {
                            let f = props.filter.clone();
                            let cb = props.on_change.clone();
                            move |e: Event<FormData>| {
                                let mut nf = f.clone();
                                nf.t_start_s = e.value();
                                cb.call(nf);
                            }
                        },
                    }
                    span { class: "filter-sep", "-" }
                    input {
                        r#type: "number",
                        placeholder: "End",
                        value: "{props.filter.t_end_s}",
                        min: "0",
                        class: "filter-input",
                        oninput: {
                            let f = props.filter.clone();
                            let cb = props.on_change.clone();
                            move |e: Event<FormData>| {
                                let mut nf = f.clone();
                                nf.t_end_s = e.value();
                                cb.call(nf);
                            }
                        },
                    }
                }
            }

            // mac filter
            div { class: "filter-group",
                span { class: "filter-label", "MAC Address" }
                span { class: "filter-hint", "Substirng match" }
                input {
                    r#type: "text",
                    placeholder: "e.g. aa:bb:cc",
                    value: "{props.filter.mac}",
                    class: "filter-input wide",
                    oninput: {
                        let f = props.filter.clone();
                        let cb = props.on_change.clone();
                        move |e: Event<FormData>| {
                            let mut nf = f.clone();
                            nf.mac = e.value();
                            cb.call(nf);
                        }
                    },
                }
            }

            // vehicle id filter
            div { class: "filter-group",
                span { class: "filter-label", "Vehicle ID" }
                span { class: "filter-hint", "Restricts to vehicle's MAC chain" }
                select {
                    class: "filter-select",
                    onchange: {
                        let f = props.filter.clone();
                        let cb = props.on_change.clone();
                        move |e: Event<FormData>| {
                            let mut nf = f.clone();
                            nf.vehicle_id = e.value().parse::<u32>().ok();
                            cb.call(nf);
                        }
                    },
                    option {
                        value: "",
                        selected: props.filter.vehicle_id.is_none(),
                        "-- All vehicles --"
                    }
                    for (vid , label) in veh_opts.iter() {
                        option {
                            key: "{vid}",
                            value: "{vid}",
                            selected: props.filter.vehicle_id == Some(*vid),
                            "{label}"
                        }
                    }
                }
            }

            // vehicle length filter
            div { class: "filter-group",
                span { class: "filter-label", "Vehicle Length (m)" }
                span { class: "filter-hint", "Target +/- tolerance" }
                div { class: "filter-inline",
                    input {
                        r#type: "number",
                        placeholder: "e.g. 4.5",
                        value: "{props.filter.veh_len}",
                        step: "0.1",
                        class: "filter-input",
                        oninput: {
                            let f = props.filter.clone();
                            let cb = props.on_change.clone();
                            move |e: Event<FormData>| {
                                let mut nf = f.clone();
                                nf.veh_len = e.value();
                                cb.call(nf);
                            }
                        },
                    }
                    span { class: "filter-sep", "+/-" }
                    input {
                        r#type: "number",
                        placeholder: "0.5",
                        value: "{props.filter.veh_len_tol}",
                        step: "0.1",
                        class: "filter-input narrow",
                        oninput: {
                            let f = props.filter.clone();
                            let cb = props.on_change.clone();
                            move |e: Event<FormData>| {
                                let mut nf = f.clone();
                                nf.veh_len_tol = e.value();
                                cb.call(nf);
                            }
                        },
                    }
                    span { class: "filter-sep", "m" }
                }
            }

            // clear button
            button {
                class: "btn btn-danger",
                onclick: move |_| props.on_clear.call(()),
                "Clear"
            }
        }
    }
}

// helpers

fn fmt_duration(min_ms: i64, max_ms: i64) -> String {
    let secs = ((max_ms - min_ms) / 1000).max(0);
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

#[derive(Props, Clone, PartialEq)]
struct StatPillProps {
    label: String,
    value: String,
}

#[component]
fn stat_pill(props: StatPillProps) -> Element {
    rsx! {
        div { class: "stat-pill",
            "{props.label}: "
            strong { "{props.value}" }
        }
    }
}
