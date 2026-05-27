use dioxus::prelude::*;
use platform::stats::MacTimelinePoint;
use chrono::{TimeZone, Utc};

// Tab20-inspired pallete - 20 visually distinct colours
const COLORS: &[&str] = &[
    "#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd",
    "#8c564b", "#e377c2", "#7f7f7f", "#bcbd22", "#17becf",
    "#aec7e8", "#ffbb78", "#98df8a", "#ff9896", "#c5b0d5",
    "#c49c94", "#f7b6d2", "#c7c7c7", "#dbdb8d", "#9edae5",
];

// Dark-theme SVG palette constants.
const BG_CHART:   &str = "#1a2030";
const BG_ROW_ALT: &str = "rgba(255,255,255,0.04)";
const BORDER:     &str = "#2a3244";
const AXIS:       &str = "#3a4460";
const LABEL:      &str = "#6a7a99";

#[derive(Props, PartialEq, Clone)]
pub struct MacTimelineProps {
    pub points: Vec<MacTimelinePoint>,
}

/// Scatter-plot chart: one row per MAC address, time on the x-axis.
///
/// Each dot marks a second during which at least one packet from that MAC was
/// received. Colours cycle through a 20-colour tab20 palette so every MAC is
/// visually distinct. Pure display component — owns no side effects.
#[component]
pub fn MacTimeline(props: MacTimelineProps) -> Element {
    if props.points.is_empty() {
        return rsx! {
            p { class: "timeline-empty", "No data yet — waiting for packets..." }
        };
    }

    // Unique MACs in first-seen order.
    let mut macs: Vec<String> = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for p in &props.points {
            if seen.insert(p.mac.clone()) {
                macs.push(p.mac.clone());
            }
        }
    }

    let n_macs = macs.len();

    // Layout constants (SVG units = px at 1:1).
    let left_m:   f64 = 155.0;
    let right_m:  f64 = 16.0;
    let top_m:    f64 = 8.0;
    let bottom_m: f64 = 36.0;
    let row_h:    f64 = 22.0;
    let svg_w:    f64 = 900.0;
    let chart_w:  f64 = svg_w - left_m - right_m;
    let chart_h:  f64 = n_macs as f64 * row_h;
    let svg_h:    f64 = top_m + chart_h + bottom_m;

    let min_ts  = props.points.iter().map(|p| p.timestamp_ms).min().unwrap_or(0);
    let max_ts  = props.points.iter().map(|p| p.timestamp_ms).max().unwrap_or(min_ts + 1_000);
    let ts_span = (max_ts - min_ts).max(1) as f64;

    let x_of = |ts: i64| -> f64 { left_m + (ts - min_ts) as f64 / ts_span * chart_w };
    let y_of = |idx: usize| -> f64 { top_m + idx as f64 * row_h + row_h * 0.5 };

    // Five evenly spaced x-axis tick labels.
    let x_ticks: Vec<(f64, String)> = (0..=4)
        .map(|i| {
            let ts    = min_ts + i as i64 * (max_ts - min_ts) / 4;
            let label = Utc.timestamp_millis_opt(ts).unwrap().format("%H:%M:%S").to_string();
            (x_of(ts), label)
        })
        .collect();

    rsx! {
        div { style: "overflow-x: auto; overflow-y: auto;",
            svg {
                width: "{svg_w}",
                height: "{svg_h}",
                view_box: "0 0 {svg_w} {svg_h}",
                style: "display: block;",

                // Chart background
                rect {
                    x: "{left_m}",
                    y: "{top_m}",
                    width: "{chart_w}",
                    height: "{chart_h}",
                    fill: BG_CHART,
                    stroke: BORDER,
                    stroke_width: "1",
                }

                // Alternating row shading
                for (i , _) in macs.iter().enumerate() {
                    if i % 2 == 0 {
                        rect {
                            key: "row-bg-{i}",
                            x: "{left_m}",
                            y: "{top_m + i as f64 * row_h}",
                            width: "{chart_w}",
                            height: "{row_h}",
                            fill: BG_ROW_ALT,
                        }
                    }
                }

                // Y-axis labels
                for (i , mac) in macs.iter().enumerate() {
                    text {
                        key: "ylabel-{i}",
                        x: "{left_m - 6.0}",
                        y: "{y_of(i) + 4.0}",
                        "text-anchor": "end",
                        "font-size": "10",
                        "font-family": "monospace",
                        fill: "{COLORS[i % COLORS.len()]}",
                        "{mac}"
                    }
                }

                // Data points
                for point in props.points.iter() {
                    if let Some(mac_idx) = macs.iter().position(|m| m == &point.mac) {
                        circle {
                            key: "pt-{point.timestamp_ms}-{point.mac}",
                            cx: "{x_of(point.timestamp_ms)}",
                            cy: "{y_of(mac_idx)}",
                            r: "2.5",
                            fill: "{COLORS[mac_idx % COLORS.len()]}",
                            opacity: "0.85",
                        }
                    }
                }

                // X-axis baseline
                line {
                    x1: "{left_m}",
                    y1: "{top_m + chart_h}",
                    x2: "{left_m + chart_w}",
                    y2: "{top_m + chart_h}",
                    stroke: AXIS,
                    stroke_width: "1",
                }

                // X-axis ticks and labels
                for (tx , label) in x_ticks.iter() {
                    line {
                        key: "xtick-{label}",
                        x1: "{tx}",
                        y1: "{top_m + chart_h}",
                        x2: "{tx}",
                        y2: "{top_m + chart_h + 5.0}",
                        stroke: AXIS,
                        stroke_width: "1",
                    }
                    text {
                        key: "xlabel-{label}",
                        x: "{tx}",
                        y: "{top_m + chart_h + 18.0}",
                        "text-anchor": "middle",
                        "font-size": "10",
                        fill: LABEL,
                        "{label}"
                    }
                }
            }
        }
    }
}
