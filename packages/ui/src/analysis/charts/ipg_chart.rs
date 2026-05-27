use dioxus::prelude::*;
use super::{IpgStats, COLORS};

#[derive(Props, Clone, PartialEq)]
pub struct IpgChartProps {
    pub stats: Vec<IpgStats>,
}

#[component]
pub fn IpgBoxPlot(props: IpgChartProps) -> Element {
    if props.stats.is_empty() {
        return rsx! { p { style: "color: #999; font-size: 0.85rem;", "Insufficient data for inter-pakect gap analysis." } };
    }

    let n = props.stats.len();
    let row_h = 32.0_f64;
    let box_h = 14.0_f64;
    let left_m = 138.0_f64;
    let right_m = 18.0_f64;
    let top_m = 14.0_f64;
    let bottom_m = 32.0_f64;
    let w = 520.0_f64;
    let h = top_m + n as f64 * row_h + bottom_m;
    let pw = w - left_m - right_m;

    let x_max = props.stats.iter().map(|s| s.whisker_hi_ms).fold(0.0_f64, f64::max).min(250.0).max(20.0);
    let xf = |v: f64| left_m + (v / x_max).min(1.0) * pw;
    let yc = |i: usize| top_m + i as f64 * row_h + row_h * 0.5;

    let x_ticks: Vec<(f64, String)> = (0..=5).map(|i| {
        let v = i as f64 / 5.0 * x_max;
        (xf(v), format!("{:.0}", v))
    }).collect();

    rsx! {
        div { style: "overflow-x: auto;",
            svg {
                width: "100%",
                height: "{h}",
                view_box: "0 0 {w} {h}",
                style: "display: block;",

                for (tx, _) in x_ticks.iter() {
                    line { key: "g{tx}", x1: "{tx}", y1: "{top_m}", x2: "{tx}", y2: "{top_m + n as f64 * row_h}", stroke: "#eee", stroke_width: "1" }
                }

                for (i, s) in props.stats.iter().enumerate() {
                    {
                        let c = yc(i);
                        let yb = c - box_h / 2.0;
                        let col = COLORS[i % COLORS.len()];
                        rsx! {
                            line { key: "wl{i}", x1: "{xf(s.whisker_lo_ms)}", y1: "{c}", x2: "{xf(s.whisker_hi_ms)}", y2: "{c}", stroke: col, stroke_width: "1.5" }
                            line { key: "cl{i}", x1: "{xf(s.whisker_lo_ms)}", y1: "{c-5.0}", x2: "{xf(s.whisker_lo_ms)}", y2: "{c+5.0}", stroke: col, stroke_width: "1.5" }
                            line { key: "ch{i}", x1: "{xf(s.whisker_hi_ms)}", y1: "{c-5.0}", x2: "{xf(s.whisker_hi_ms)}", y2: "{c+5.0}", stroke: col, stroke_width: "1.5" }
                            rect {
                                key: "box{i}",
                                x: "{xf(s.q1_ms)}", y: "{yb}",
                                width: "{(xf(s.q3_ms) - xf(s.q1_ms)).max(2.0)}", height: "{box_h}",
                                fill: col, fill_opacity: "0.2", stroke: col, stroke_width: "1.5",
                            }
                            line { key: "med{i}", x1: "{xf(s.median_ms)}", y1: "{yb}", x2: "{xf(s.median_ms)}", y2: "{yb + box_h}", stroke: col, stroke_width: "2.5" }
                            text {
                                key: "lbl{i}",
                                x: "{left_m - 6.0}", y: "{c + 4.0}",
                                "text-anchor": "end", "font-size": "10", "font-family": "monospace",
                                fill: col,
                                "{s.mac}"
                            }
                        }
                    }
                }

                line { x1: "{left_m}", y1: "{top_m + n as f64 * row_h}", x2: "{left_m + pw}", y2: "{top_m + n as f64 * row_h}", stroke: "#aaa", stroke_width: "1" }

                for (tx, lbl) in x_ticks.iter() {
                    line { key: "xt{lbl}", x1: "{tx}", y1: "{top_m + n as f64 * row_h}", x2: "{tx}", y2: "{top_m + n as f64 * row_h + 4.0}", stroke: "#aaa", stroke_width: "1" }
                    text { key: "xl{lbl}", x: "{tx}", y: "{top_m + n as f64 * row_h + 16.0}", "text-anchor": "middle", "font-size": "10", fill: "#666", "{lbl}" }
                }
                text { x: "{left_m + pw / 2.0}", y: "{h - 2.0}", "text-anchor": "middle", "font-size": "10", fill: "#555", "Inter-Packet Gap (ms)" }
            }
        }
    }
}
