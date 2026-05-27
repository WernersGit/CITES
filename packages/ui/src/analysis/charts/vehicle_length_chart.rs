use dioxus::prelude::*;
use super::{VehicleLengthSample, COLORS, fmt_hms};

#[derive(Props, Clone, PartialEq)]
pub struct VehicleLengthChartProps {
    pub samples: Vec<VehicleLengthSample>,
    pub mac_order: Vec<String>,
    pub min_ms: i64,
    pub max_ms: i64,
}

/// Vehicle Length Profile over time — matches Python CamAnalyzer.py report 7.
/// X = capture timestmap, Y = vehicle length in metres.
/// One connected line + dots per MAC, sorted chronologically.
#[component]
pub fn VehicleLengthChart(props: VehicleLengthChartProps) -> Element {
    if props.samples.is_empty() {
        return rsx! { p { style: "color: #999; font-size: 0.85rem;", "No vehicle length data available (requires decoded CAM payload)." } };
    }

    let w = 520.0_f64;
    let h = 240.0_f64;
    let lm = 50.0_f64;
    let rm = 12.0_f64;
    let tm = 12.0_f64;
    let bm = 44.0_f64;
    let pw = w - lm - rm;
    let ph = h - tm - bm;

    let ts_span = (props.max_ms - props.min_ms).max(1) as f64;
    let y_max = (props.samples.iter().map(|s| s.length_m).fold(0.0_f64, f64::max) * 1.15).max(6.0).ceil();

    let xf = |ms: i64| lm + (ms - props.min_ms) as f64 / ts_span * pw;
    let yf = |v: f64| tm + ph - (v / y_max) * ph;

    let macs_with_data: Vec<&String> = props.mac_order.iter()
        .filter(|m| props.samples.iter().any(|s| &s.mac == *m))
        .collect();

    let x_ticks: Vec<(f64, String)> = (0..=4).map(|i| {
        let ms = props.min_ms + i as i64 * (props.max_ms - props.min_ms) / 4;
        (xf(ms), fmt_hms(ms))
    }).collect();
    let y_ticks: Vec<(f64, String)> = (0..=4).map(|i| {
        let v = i as f64 / 4.0 * y_max;
        (yf(v), format!("{:.1}", v))
    }).collect();

    let stride = (props.samples.len() / 6000).max(1);

    rsx! {
        svg {
            width: "100%",
            height: "{h}",
            view_box: "0 0 {w} {h}",
            style: "display: block;",

            for (ty, _) in y_ticks.iter() {
                line { key: "yg{ty}", x1: "{lm}", y1: "{ty}", x2: "{lm + pw}", y2: "{ty}", stroke: "#eee", stroke_width: "1" }
            }

            for (mi, mac) in macs_with_data.iter().enumerate() {
                {
                    let col = COLORS[mi % COLORS.len()];
                    let mac_samples: Vec<&VehicleLengthSample> = props.samples.iter()
                        .filter(|s| &s.mac == *mac)
                        .step_by(stride)
                        .collect();

                    if mac_samples.len() >= 2 {
                        let d: String = mac_samples.iter().enumerate().map(|(j, s)| {
                            let x = xf(s.timestamp_ms);
                            let y = yf(s.length_m);
                            if j == 0 { format!("M {:.1} {:.1}", x, y) } else { format!("L {:.1} {:.1}", x, y) }
                        }).collect::<Vec<_>>().join(" ");
                        rsx! {
                            path { key: "line{mi}", d: "{d}", stroke: col, stroke_width: "1.2", fill: "none", opacity: "0.7" }
                            for s in mac_samples.iter() {
                                circle {
                                    key: "dot{s.timestamp_ms}{mi}",
                                    cx: "{xf(s.timestamp_ms)}", cy: "{yf(s.length_m)}",
                                    r: "1.8", fill: col, opacity: "0.6",
                                }
                            }
                        }
                    } else {
                        rsx! {
                            for s in mac_samples.iter() {
                                circle {
                                    key: "dot{s.timestamp_ms}{mi}",
                                    cx: "{xf(s.timestamp_ms)}", cy: "{yf(s.length_m)}",
                                    r: "2.5", fill: col, opacity: "0.7",
                                }
                            }
                        }
                    }
                }
            }

            line { x1: "{lm}", y1: "{tm}", x2: "{lm}", y2: "{tm + ph}", stroke: "#aaa", stroke_width: "1" }
            line { x1: "{lm}", y1: "{tm + ph}", x2: "{lm + pw}", y2: "{tm + ph}", stroke: "#aaa", stroke_width: "1" }

            for (ty, lbl) in y_ticks.iter() {
                line { key: "yt{lbl}", x1: "{lm - 4.0}", y1: "{ty}", x2: "{lm}", y2: "{ty}", stroke: "#aaa", stroke_width: "1" }
                text { key: "yl{lbl}", x: "{lm - 6.0}", y: "{ty + 4.0}", "text-anchor": "end", "font-size": "10", fill: "#666", "{lbl}" }
            }
            text { x: "10", y: "{tm + ph / 2.0}", "text-anchor": "middle", "font-size": "10", fill: "#666",
                "transform": "rotate(-90,10,{tm + ph / 2.0})", "Vehicel Length (m)"
            }

            for (tx, lbl) in x_ticks.iter() {
                line { key: "xt{lbl}", x1: "{tx}", y1: "{tm + ph}", x2: "{tx}", y2: "{tm + ph + 4.0}", stroke: "#aaa", stroke_width: "1" }
                text { key: "xl{lbl}", x: "{tx}", y: "{tm + ph + 16.0}", "text-anchor": "middle", "font-size": "9", fill: "#666", "{lbl}" }
            }

            for (mi, mac) in macs_with_data.iter().take(8).enumerate() {
                rect { key: "lr{mi}", x: "{lm + mi as f64 * 65.0}", y: "{h - 10.0}", width: "9", height: "4", fill: "{COLORS[mi % COLORS.len()]}", opacity: "0.85" }
                text { key: "lt{mi}", x: "{lm + mi as f64 * 65.0 + 13.0}", y: "{h - 4.0}", "font-size": "8", "font-family": "monospace", fill: "#444", "{mac}" }
            }
        }
    }
}
