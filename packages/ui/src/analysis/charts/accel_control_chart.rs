use dioxus::prelude::*;
use super::{AccelControlEvent, COLORS, fmt_hms};

const FLAG_BITS: &[(u8, &str)] = &[
    (1 << 7, "Brakes"),
    (1 << 6, "Accelerator"),
    (1 << 5, "Emergency Braking"),
    (1 << 4, "Collision Warning"),
    (1 << 3, "ACC"),
    (1 << 2, "Cruise Control"),
    (1 << 1, "Speed Limiter"),
];

#[derive(Props, Clone, PartialEq)]
pub struct AccelControlChartProps {
    pub events: Vec<AccelControlEvent>,
    pub mac_order: Vec<String>,
    pub min_ms: i64,
    pub max_ms: i64,
}

/// Acceleration Control Flags timeline — matches Python CamAnalyzer.py report 8.
///
/// One row per flag on the Y-axis; time on the X-axis.
/// Each active-flag event is drawn as a thin veritcal tick (`|`) coloured by MAC.
#[component]
pub fn AccelControlChart(props: AccelControlChartProps) -> Element {
    let macs_with_data: Vec<&String> = props.mac_order.iter()
        .filter(|m| props.events.iter().any(|e| &e.mac == *m))
        .take(8)
        .collect();

    if macs_with_data.is_empty() {
        return rsx! { p { style: "color: #999; font-size: 0.85rem;",
            "No accelerationControl data available (requires decoded CAM payload)." } };
    }

    let n_flags = FLAG_BITS.len();
    let row_h = 28.0_f64;
    let lm = 110.0_f64;
    let rm = 12.0_f64;
    let tm = 8.0_f64;
    let bm = 52.0_f64;
    let w = 520.0_f64;
    let h = tm + n_flags as f64 * row_h + bm;
    let pw = w - lm - rm;

    let ts_span = (props.max_ms - props.min_ms).max(1) as f64;
    let xf = |ms: i64| lm + (ms - props.min_ms) as f64 / ts_span * pw;
    let yc = |row: usize| tm + row as f64 * row_h + row_h * 0.5;

    let tick_h = row_h * 0.55;
    let stride = (props.events.len() / 4000).max(1);

    let x_ticks: Vec<(f64, String)> = (0..=4).map(|i| {
        let ms = props.min_ms + i as i64 * (props.max_ms - props.min_ms) / 4;
        (xf(ms), fmt_hms(ms))
    }).collect();

    rsx! {
        div { style: "overflow-x: auto;",
            svg {
                width: "100%",
                height: "{h}",
                view_box: "0 0 {w} {h}",
                style: "display: block;",

                for row in 0..n_flags {
                    line {
                        key: "hg{row}",
                        x1: "{lm}", y1: "{yc(row)}", x2: "{lm + pw}", y2: "{yc(row)}",
                        stroke: "#eee", stroke_width: "1",
                    }
                }

                for (mi, mac) in macs_with_data.iter().enumerate() {
                    {
                        let col = COLORS[mi % COLORS.len()];
                        let mac_events: Vec<&AccelControlEvent> = props.events.iter()
                            .filter(|e| &e.mac == *mac)
                            .step_by(stride)
                            .collect();
                        rsx! {
                            for ev in mac_events.iter() {
                                for (row, (mask, _)) in FLAG_BITS.iter().enumerate() {
                                    if ev.flags & mask != 0 {
                                        {
                                            let cx = xf(ev.timestamp_ms);
                                            let cy = yc(row);
                                            rsx! {
                                                line {
                                                    key: "ev{ev.timestamp_ms}{mi}{row}",
                                                    x1: "{cx}", y1: "{cy - tick_h / 2.0}",
                                                    x2: "{cx}", y2: "{cy + tick_h / 2.0}",
                                                    stroke: col, stroke_width: "1.5", opacity: "0.7",
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                for (row, (_, name)) in FLAG_BITS.iter().enumerate() {
                    text {
                        key: "fl{row}",
                        x: "{lm - 6.0}", y: "{yc(row) + 4.0}",
                        "text-anchor": "end", "font-size": "10", fill: "#444",
                        "{name}"
                    }
                }

                line { x1: "{lm}", y1: "{tm}", x2: "{lm}", y2: "{tm + n_flags as f64 * row_h}", stroke: "#aaa", stroke_width: "1" }
                line { x1: "{lm}", y1: "{tm + n_flags as f64 * row_h}", x2: "{lm + pw}", y2: "{tm + n_flags as f64 * row_h}", stroke: "#aaa", stroke_width: "1" }

                for (tx, lbl) in x_ticks.iter() {
                    line { key: "xt{lbl}", x1: "{tx}", y1: "{tm + n_flags as f64 * row_h}", x2: "{tx}", y2: "{tm + n_flags as f64 * row_h + 4.0}", stroke: "#aaa", stroke_width: "1" }
                    text { key: "xl{lbl}", x: "{tx}", y: "{tm + n_flags as f64 * row_h + 16.0}", "text-anchor": "middle", "font-size": "9", fill: "#666", "{lbl}" }
                }

                for (mi, mac) in macs_with_data.iter().enumerate() {
                    rect { key: "lr{mi}", x: "{lm + mi as f64 * 65.0}", y: "{h - 12.0}", width: "9", height: "9", fill: "{COLORS[mi % COLORS.len()]}", opacity: "0.85" }
                    text { key: "lt{mi}", x: "{lm + mi as f64 * 65.0 + 13.0}", y: "{h - 3.0}", "font-size": "8", "font-family": "monospace", fill: "#444", "{mac}" }
                }
            }
        }
    }
}
