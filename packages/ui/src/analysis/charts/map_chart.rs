use dioxus::prelude::*;
use std::collections::HashMap;
use platform::ConnectionService;
use crate::countries::find_country;
use crate::trajectory::MacTrajectory;
use super::{VehicleRow, COLORS};

const MAPLIBRE_JS: Asset  = asset!("/assets/maplibre-gl.js");
const MAPLIBRE_CSS: Asset = asset!("/assets/maplibre-gl.css");

// GeoJSON builder

/// builds the traj JSON for MapLibre GL — each etry has mac/label/color/points
fn build_traj_json(trajectories: &[MacTrajectory], vehicles: &[VehicleRow]) -> String {
    let mut mac_to_vid: HashMap<&str, i64> = HashMap::new();
    for row in vehicles {
        for mac in &row.macs {
            mac_to_vid.insert(mac.as_str(), row.virtual_id as i64);
        }
    }

    // Group trajectories by vehicle ID for color assignment.
    let mut vid_trajs: HashMap<i64, Vec<&MacTrajectory>> = HashMap::new();
    for traj in trajectories {
        let vid = mac_to_vid.get(traj.mac.as_str()).copied().unwrap_or(-1);
        vid_trajs.entry(vid).or_default().push(traj);
    }
    let mut vids: Vec<i64> = vid_trajs.keys().copied().collect();
    vids.sort();

    let mut entries: Vec<String> = Vec::new();
    for (color_idx, vid) in vids.iter().enumerate() {
        let color = COLORS[color_idx % COLORS.len()];
        let label = match vid {
            0 => "Ego".to_string(),
            v if *v > 0 => format!("Vehicle {}", v),
            _ => "Unknown".to_string(),
        };
        for traj in &vid_trajs[vid] {
            let pts = &traj.points;
            if pts.len() < 2 {
                continue;
            }
            // Point sampling: keep first/last 20, sample the middle.
            const ANCHOR: usize = 20;
            const MID_CAP: usize = 3000;
            let pts_json: String = if pts.len() <= ANCHOR * 2 {
                pts.iter()
                    .map(|(lon, lat)| format!("[{:.6},{:.6}]", lon, lat))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                let head = &pts[..ANCHOR];
                let tail = &pts[pts.len() - ANCHOR..];
                let mid  = &pts[ANCHOR..pts.len() - ANCHOR];
                let stride = (mid.len() / MID_CAP).max(1);
                let mut out = Vec::with_capacity(ANCHOR * 2 + mid.len() / stride + 1);
                for (lon, lat) in head { out.push(format!("[{:.6},{:.6}]", lon, lat)); }
                for (lon, lat) in mid.iter().step_by(stride) { out.push(format!("[{:.6},{:.6}]", lon, lat)); }
                for (lon, lat) in tail { out.push(format!("[{:.6},{:.6}]", lon, lat)); }
                out.join(",")
            };
            let mac_esc = traj.mac.replace('"', "\\\"");
            let lbl_esc = label.replace('"', "\\\"");
            entries.push(format!(
                r#"{{"mac":"{mac_esc}","label":"{lbl_esc}","color":"{color}","points":[{pts_json}]}}"#
            ));
        }
    }
    format!("[{}]", entries.join(","))
}

// JS builder

fn make_init_js(style_url: &str, traj_json: &str, center_lon: f64, center_lat: f64, center_zoom: u8) -> String {
    let style_url = style_url.replace('\'', "\\'");
    let js_url  = MAPLIBRE_JS.to_string();
    let css_url = MAPLIBRE_CSS.to_string();
    format!(r#"(async function() {{
    if (!document.getElementById('_cites_mgl_css')) {{
        var lnk = document.createElement('link');
        lnk.id  = '_cites_mgl_css';
        lnk.rel = 'stylesheet';
        lnk.href = '{css_url}';
        document.head.appendChild(lnk);
    }}
    if (!window.maplibregl) {{
        await new Promise(function(ok, err) {{
            var s = document.createElement('script');
            s.src = '{js_url}';
            s.onload = ok; s.onerror = err;
            document.head.appendChild(s);
        }});
    }}
    var el = document.getElementById('analysis-map-container');
    if (!el) return;
    if (window._analysisMap) {{ window._analysisMap.remove(); window._analysisMap = null; }}

    var map = new maplibregl.Map({{
        container: 'analysis-map-container',
        style: '{style_url}',
        center: [{center_lon}, {center_lat}],
        zoom: {center_zoom},
        attributionControl: false
    }});
    window._analysisMap = map;

    map.addControl(new maplibregl.NavigationControl(), 'top-right');
    map.addControl(new maplibregl.ScaleControl({{ unit: 'metric' }}), 'bottom-left');

    var vehicles = {traj_json};

    map.on('load', function() {{
        var allCoords = [];

        vehicles.forEach(function(v) {{
            if (!v.points || v.points.length < 2) return;
            var sid = v.mac.replace(/[^a-zA-Z0-9_-]/g, '_');
            allCoords = allCoords.concat(v.points);

            map.addSource('traj-' + sid, {{
                type: 'geojson',
                data: {{ type: 'Feature', properties: {{}}, geometry: {{ type: 'LineString', coordinates: v.points }} }}
            }});
            map.addLayer({{
                id: 'traj-line-' + sid,
                type: 'line',
                source: 'traj-' + sid,
                layout: {{ 'line-join': 'round', 'line-cap': 'round' }},
                paint: {{ 'line-color': v.color, 'line-width': 3, 'line-opacity': 0.85 }}
            }});

            map.addSource('start-' + sid, {{
                type: 'geojson',
                data: {{
                    type: 'Feature',
                    properties: {{ label: v.label + ' — ' + v.mac }},
                    geometry: {{ type: 'Point', coordinates: v.points[0] }}
                }}
            }});
            map.addLayer({{
                id: 'start-dot-' + sid,
                type: 'circle',
                source: 'start-' + sid,
                paint: {{
                    'circle-radius': 7,
                    'circle-color': v.color,
                    'circle-stroke-width': 2,
                    'circle-stroke-color': '#ffffff',
                    'circle-opacity': 1
                }}
            }});

            map.on('mouseenter', 'traj-line-' + sid, function() {{ map.getCanvas().style.cursor = 'pointer'; }});
            map.on('mouseleave', 'traj-line-' + sid, function() {{ map.getCanvas().style.cursor = ''; }});
        }});

        if (allCoords.length > 0) {{
            var bounds = allCoords.reduce(function(b, c) {{
                return b.extend(c);
            }}, new maplibregl.LngLatBounds(allCoords[0], allCoords[0]));
            map.fitBounds(bounds, {{ padding: 40, maxZoom: 17 }});
        }}
    }});
}})();"#,
        style_url = style_url,
        traj_json = traj_json,
        center_lon = center_lon,
        center_lat = center_lat,
        center_zoom = center_zoom,
    )
}

// component

#[derive(Props, Clone, PartialEq)]
pub struct MapChartProps {
    pub trajectories: Vec<MacTrajectory>,
    pub vehicles: Vec<VehicleRow>,
}

#[component]
pub fn MapChart(props: MapChartProps) -> Element {
    let conn = use_context::<ConnectionService>();
    let base_url = conn.live_tile_server_url.read().clone();
    let sty = crate::style_url(&base_url);
    let cty = find_country(&conn.country_code.read());

    let js = make_init_js(
        &sty,
        &build_traj_json(&props.trajectories, &props.vehicles),
        cty.lon,
        cty.lat,
        cty.zoom,
    );

    let mut js_signal = use_signal(|| js.clone());
    let changed = *js_signal.read() != js;
    if changed {
        js_signal.set(js);
    }

    use_effect(move || {
        let js = js_signal.read().clone();
        spawn(async move {
            let _ = document::eval(&js).await;
        });
    });

    rsx! {
        div {
            style: "width: 100%; height: 500px; border-radius: 6px; overflow: hidden;",
            div {
                id: "analysis-map-container",
                style: "width: 100%; height: 100%;",
            }
        }
    }
}
