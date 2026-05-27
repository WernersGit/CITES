use dioxus::prelude::*;
use super::COLORS;
use crate::trajectory::MacTrajectory;

#[derive(Props, Clone, PartialEq)]
pub struct TrajectoryProps {
    pub trajectories: Vec<MacTrajectory>,
    pub max_macs: Option<usize>,
}

#[component]
pub fn TrajectoryChart(props: TrajectoryProps) -> Element {
    if props.trajectories.is_empty() {
        return rsx! { p { style: "color: #999; font-size: 0.85rem;", "No position data availble (latitude/longitude flagged unavailable in GeoNetworking header)." } };
    }

    let size = 460.0_f64;
    let lm = 54.0_f64;
    let rm = 12.0_f64;
    let tm = 12.0_f64;
    let bm = 40.0_f64;
    let pw = size - lm - rm;
    let ph = size - tm - bm;

    let active: Vec<&MacTrajectory> = match props.max_macs {
        Some(n) => props.trajectories.iter().take(n).collect(),
        None    => props.trajectories.iter().collect(),
    };

    // bounding box - points are (lon, lat)
    let min_lat = active.iter().flat_map(|t| t.points.iter()).map(|(_, lat)| *lat).fold(f64::INFINITY, f64::min);
    let max_lat = active.iter().flat_map(|t| t.points.iter()).map(|(_, lat)| *lat).fold(f64::NEG_INFINITY, f64::max);
    let min_lon = active.iter().flat_map(|t| t.points.iter()).map(|(lon, _)| *lon).fold(f64::INFINITY, f64::min);
    let max_lon = active.iter().flat_map(|t| t.points.iter()).map(|(lon, _)| *lon).fold(f64::NEG_INFINITY, f64::max);

    let lat_span = (max_lat - min_lat).max(1e-5) * 1.10;
    let lon_span = (max_lon - min_lon).max(1e-5) * 1.10;
    let lat_center = (min_lat + max_lat) / 2.0;
    let lon_center = (min_lon + max_lon) / 2.0;
    let aspect = lat_span / lon_span;
    let (lat_range, lon_range) = if aspect > ph / pw {
        let lr = lat_span;
        (lr, lr * pw / ph)
    } else {
        let lr = lon_span * ph / pw;
        (lr, lon_span)
    };
    let lat_lo = lat_center - lat_range / 2.0;
    let lat_hi = lat_center + lat_range / 2.0;
    let lon_lo = lon_center - lon_range / 2.0;
    let lon_hi = lon_center + lon_range / 2.0;

    let xf = |lon: f64| lm + (lon - lon_lo) / (lon_hi - lon_lo) * pw;
    let yf = |lat: f64| tm + (1.0 - (lat - lat_lo) / (lat_hi - lat_lo)) * ph;

    let y_ticks: Vec<(f64, String)> = (0..=4).map(|i| {
        let v = lat_lo + i as f64 / 4.0 * lat_range;
        (yf(v), format!("{:.4}°", v))
    }).collect();
    let x_ticks: Vec<(f64, String)> = (0..=3).map(|i| {
        let v = lon_lo + i as f64 / 3.0 * lon_range;
        (xf(v), format!("{:.4}°", v))
    }).collect();

    rsx! {
        svg {
            width: "100%",
            height: "{size}",
            view_box: "0 0 {size} {size}",
            style: "display: block;",

            for (ty, _) in y_ticks.iter() {
                line { key: "yg{ty}", x1: "{lm}", y1: "{ty}", x2: "{lm + pw}", y2: "{ty}", stroke: "#eee", stroke_width: "1" }
            }
            for (tx, _) in x_ticks.iter() {
                line { key: "xg{tx}", x1: "{tx}", y1: "{tm}", x2: "{tx}", y2: "{tm + ph}", stroke: "#eee", stroke_width: "1" }
            }

            for (mi, traj) in active.iter().enumerate() {
                {
                    let col = COLORS[mi % COLORS.len()];
                    let total = traj.points.len();
                    let stride = (total / 8000).max(1);
                    let pts: Vec<(f64, f64)> = traj.points.iter().step_by(stride).copied().collect();
                    if pts.len() >= 2 {
                        let d: String = pts.iter().enumerate().map(|(j, (lon, lat))| {
                            let x = xf(*lon);
                            let y = yf(*lat);
                            if j == 0 { format!("M {:.1} {:.1}", x, y) } else { format!("L {:.1} {:.1}", x, y) }
                        }).collect::<Vec<_>>().join(" ");
                        let (first_lon, first_lat) = pts[0];
                        rsx! {
                            path { key: "path{mi}", d: "{d}", stroke: col, stroke_width: "1.5", fill: "none", opacity: "0.8" }
                            circle { key: "s{mi}", cx: "{xf(first_lon)}", cy: "{yf(first_lat)}", r: "4", fill: col, stroke: "#fff", stroke_width: "1" }
                        }
                    } else if let Some((lon, lat)) = pts.first() {
                        rsx! { circle { key: "dot{mi}", cx: "{xf(*lon)}", cy: "{yf(*lat)}", r: "4", fill: col, opacity: "0.85" } }
                    } else {
                        rsx! {}
                    }
                }
            }

            line { x1: "{lm}", y1: "{tm}", x2: "{lm}", y2: "{tm + ph}", stroke: "#aaa", stroke_width: "1" }
            line { x1: "{lm}", y1: "{tm + ph}", x2: "{lm + pw}", y2: "{tm + ph}", stroke: "#aaa", stroke_width: "1" }

            for (ty, lbl) in y_ticks.iter() {
                line { key: "yt{lbl}", x1: "{lm - 4.0}", y1: "{ty}", x2: "{lm}", y2: "{ty}", stroke: "#aaa", stroke_width: "1" }
                text { key: "yl{lbl}", x: "{lm - 6.0}", y: "{ty + 4.0}", "text-anchor": "end", "font-size": "9", fill: "#666", "{lbl}" }
            }
            for (tx, lbl) in x_ticks.iter() {
                line { key: "xt{lbl}", x1: "{tx}", y1: "{tm + ph}", x2: "{tx}", y2: "{tm + ph + 4.0}", stroke: "#aaa", stroke_width: "1" }
                text { key: "xl{lbl}", x: "{tx}", y: "{tm + ph + 16.0}", "text-anchor": "middle", "font-size": "9", fill: "#666", "{lbl}" }
            }
        }
    }
}
