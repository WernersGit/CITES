use dioxus::prelude::*;
use platform::ConnectionService;
use crate::countries::find_country;

const MAPLIBRE_JS: Asset  = asset!("/assets/maplibre-gl.js");
const MAPLIBRE_CSS: Asset = asset!("/assets/maplibre-gl.css");

// trajectory data

#[derive(Clone, PartialEq)]
pub struct LiveTrajData {
    pub mac: String,
    pub color: String,
    /// Points in GeoJSON order: (longitude, latitude).
    pub points: Vec<(f64, f64)>,
}

// constants

/// Width in pixels of the invisible hit-target layer overlaid on each 3 px
/// trajectory line to make click tagets comfortably large.
const TRAJ_HIT_WIDTH: u8 = 16;

// js builders

fn make_live_map_js(base_url: &str, lon: f64, lat: f64, zoom: u8) -> String {
    let style_url = crate::style_url(base_url).replace('\'', "\\'");
    let js_url  = MAPLIBRE_JS.to_string();
    let css_url = MAPLIBRE_CSS.to_string();
    format!(r#"(async function() {{
try {{
    if (!document.getElementById('_cites_mgl_css')) {{
        var lnk = document.createElement('link');
        lnk.id='_cites_mgl_css'; lnk.rel='stylesheet'; lnk.href='{css_url}';
        document.head.appendChild(lnk);
    }}
    if (!window.maplibregl) {{
        await new Promise(function(ok,err) {{
            var s=document.createElement('script');
            s.src='{js_url}'; s.onload=ok; s.onerror=err;
            document.head.appendChild(s);
        }});
    }}
    var el = document.getElementById('live-map-container');
    if (!el) {{ console.error('[CITES] #live-map-container not found'); return; }}

    if (window._liveMap) {{ window._liveMap.remove(); window._liveMap=null; }}
    window._liveUpdatePlayback = null;
    window._citesNextFrame = null;
    window._liveAutoCenter = true;
    window._liveAutoFitted = false;
    window._liveSmooth     = true;
    window._liveAnimId     = 0;
    window._liveLast       = null;
    window._liveSmoothHdg  = undefined;
    var _oldCm = document.getElementById('_cites_cm');
    if (_oldCm) _oldCm.remove();
    window._citesCM = null;

    var map = new maplibregl.Map({{
        container:'live-map-container',
        style:'{style_url}',
        center:[{lon},{lat}], zoom:{zoom},
        attributionControl:false
    }});
    window._liveMap = map;

    // Pin the active trajectory tip to the camera center while AutoCenter is on.
    // easeTo emits 'move' continuously, so the line tracks the on-screen CSS marker
    // through the full 200 ms animation instead of jumping to the target up front.
    map.on('move', function() {{
        if (window._liveAutoCenter && window._liveUpdateTip) {{
            var c = map.getCenter();
            window._liveUpdateTip(c.lng, c.lat);
        }}
    }});

    // css center-marker: a dom element fixed at 50%/50% of the map container
    // in AutoCenter mode this replaces the GeoJSON marker so it never jitters
    // during camera animation -> it has no dependency on MapLibre's camera
    var _cm = document.createElement('div');
    _cm.id = '_cites_cm';
    // Matches the GeoJSON playback-marker layer exactly:
    // circle-radius:9 + circle-stroke-width:3 -> total diameter 24 px.
    _cm.style.cssText = 'position:absolute;left:50%;top:50%;' +
        'transform:translate(-50%,-50%);width:24px;height:24px;border-radius:50%;' +
        'box-sizing:border-box;border:3px solid #fff;background:#44aaff;' +
        'pointer-events:none;display:none;z-index:2;';
    el.appendChild(_cm);
    window._citesCM = _cm;

    map.on('error', function(e) {{ console.error('[CITES] map error:',e.error); }});

    // register scene / resize listeners before controls so a control error never prevents trajectories from appearing
    function consumePending() {{
        if (!map.isStyleLoaded()) return;
        if (window._pendingScene) {{
            window._applyScene(window._pendingScene);
            window._pendingScene = null;
        }}
    }}
    map.on('load',      function() {{ map.resize(); consumePending(); }});
    map.on('styledata', consumePending);

    // controls in an isolated try-catch - a failure here must not break map functionality
    try {{
        // adaptive zoom -> step shrinks as you zoom in
        // z<5 -> +-5  |  z<9 -> +-3  |  z<13 -> +-2  |  z>=13 -> +-1
        var _zc = {{
            onAdd: function(m) {{
                var c = document.createElement('div');
                c.className = 'maplibregl-ctrl maplibregl-ctrl-group';
                function _step() {{
                    var z = m.getZoom();
                    return z < 5 ? 5 : z < 9 ? 3 : z < 13 ? 2 : 1;
                }}
                function _btn(cls, lbl, sign) {{
                    var b = document.createElement('button');
                    b.className = cls; b.title = lbl;
                    b.setAttribute('aria-label', lbl);
                    b.innerHTML = '<span class="maplibregl-ctrl-icon" aria-hidden="true"></span>';
                    b.onclick = function() {{ m.zoomTo(m.getZoom() + sign * _step(), {{duration:200}}); }};
                    return b;
                }}
                c.appendChild(_btn('maplibregl-ctrl-zoom-in',  'Zoom in',   1));
                c.appendChild(_btn('maplibregl-ctrl-zoom-out', 'Zoom out', -1));
                return c;
            }},
            onRemove: function() {{}}
        }};
        map.addControl(_zc, 'top-right');
        map.addControl(new maplibregl.NavigationControl({{showZoom:false}}),'top-right');
        map.addControl(new maplibregl.ScaleControl({{unit:'metric'}}),'bottom-left');
    }} catch(ce) {{ console.warn('[CITES] control setup error:',ce); }}

    // single persistent raf loop -> rusr writes _citesNextFrame; js consumes at 60 fps
    // decouples the rust update rate from MapLibre rendering - no ipc queue buildup
    window._citesNextFrame = null;
    if (!window._citesRafRunning) {{
        window._citesRafRunning = true;
        (function rafPoll() {{
            var f = window._citesNextFrame;
            if (f && window._liveUpdatePlayback) {{
                window._citesNextFrame = null;
                window._liveUpdatePlayback(f[0], f[1], f[2], f[3]);
            }}
            requestAnimationFrame(rafPoll);
        }})();
    }}
}} catch(e) {{ console.error('[CITES] map init exception:',e); }}
}})();"#)
}

/// Builds the combined JS that upserts trajectory lines and the playback overlay.
///
/// Trajectory lines use an upsert strategy: existing MapLibre sources are updated in-place;
/// new sources/layers are created; stale ones are removed. This prevents layer teardown on
/// every data tick, eliminating the visual flash when trajectory data grows.
///
/// Playback layers (marker + traveled lines) are rebuilt only when the color structure
/// changes — i.e. when the selected vehicle or its MACs change. Routine data updates
/// (new GPS points, same vehicle) skip the teardown entirely.
fn make_scene_js(
    trajectories: &[LiveTrajData],
    playback_pts: Option<&[(f64, f64)]>,
    playback_colors: &[String],
) -> String {
    let has_pb  = playback_pts.is_some();
    let opacity = if has_pb { 0.18_f32 } else { 0.85_f32 };

    let entries: Vec<String> = trajectories.iter().map(|t| {
        let coords = t.points.iter()
            .map(|(lon, lat)| format!("[{:.6},{:.6}]", lon, lat))
            .collect::<Vec<_>>().join(",");
        let m = t.mac.replace('"', "\\\"");
        let c = t.color.replace('"', "\\\"");
        format!(r#"{{"mac":"{m}","color":"{c}","points":[{coords}]}}"#)
    }).collect();
    let traj_json = format!("[{}]", entries.join(","));

    let pb_block = match playback_pts {
        None => {
            // no playback active: clear all playback layers and state
            r#"
        window._liveUpdatePlayback = null;
        window._liveAnimId = (window._liveAnimId || 0) + 1;
        var _stale = window._livePlaybackColorCount || 0;
        for (var _i = 0; _i < _stale; _i++) {
            var _s = 'playback-traveled-' + _i;
            try { if (map.getLayer(_s))  map.removeLayer(_s);  } catch(e) {}
            try { if (map.getSource(_s)) map.removeSource(_s); } catch(e) {}
        }
        window._livePlaybackColorCount = 0;
        window._liveColorIdx = {};
        try { if (map.getLayer('playback-marker'))  map.removeLayer('playback-marker');  } catch(e) {}
        try { if (map.getSource('playback-marker')) map.removeSource('playback-marker'); } catch(e) {}"#
                .to_string()
        }
        Some(pts) => {
            let coords: String = pts.iter()
                .map(|(lon, lat)| format!("[{:.6},{:.6}]", lon, lat))
                .collect::<Vec<_>>().join(",");
            let c_json: String = playback_colors.iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\\\"")))
                .collect::<Vec<_>>().join(",");

            let mut uniq_colors: Vec<&str> = Vec::new();
            for c in playback_colors {
                if !uniq_colors.contains(&c.as_str()) {
                    uniq_colors.push(c.as_str());
                }
            }
            let color_count = uniq_colors.len();
            let init: String = uniq_colors.iter().enumerate().map(|(i, color)| {
                let color = color.replace('"', "\\\"");
                format!(r#"
        map.addSource('playback-traveled-{i}',{{type:'geojson',data:{{type:'Feature',geometry:{{type:'LineString',coordinates:[]}}}}}});
        map.addLayer({{id:'playback-traveled-{i}',type:'line',source:'playback-traveled-{i}',
            layout:{{'line-join':'round','line-cap':'round'}},
            paint:{{'line-color':'{color}','line-width':5,'line-opacity':0.95}}}});"#)
            }).collect();
            let color_map: String = uniq_colors.iter().enumerate().map(|(i, color)| {
                let color = color.replace('"', "\\\"");
                format!("\"{}\":{}", color, i)
            }).collect::<Vec<_>>().join(",");
            let first = pts.first()
                .map(|(lo, la)| format!("[{:.6},{:.6}]", lo, la))
                .unwrap_or_else(|| "[0,0]".to_string());

            format!(r#"
        // rebuild playback layers only when the vehicle or its color structure changes
        var _newIdx  = {{{color_map}}};
        var _rebuild = (window._livePlaybackColorCount || 0) !== {color_count};
        if (!_rebuild) {{
            var _ex = window._liveColorIdx || {{}};
            for (var _c in _newIdx) {{ if (_ex[_c] !== _newIdx[_c]) {{ _rebuild = true; break; }} }}
        }}
        if (_rebuild) {{
            // cancel any in-progress smooth animation for the old vehicle
            window._liveAnimId = (window._liveAnimId || 0) + 1;
            var _stale = window._livePlaybackColorCount || 0;
            for (var _i = 0; _i < _stale; _i++) {{
                var _s = 'playback-traveled-' + _i;
                try {{ if (map.getLayer(_s))  map.removeLayer(_s);  }} catch(e) {{}}
                try {{ if (map.getSource(_s)) map.removeSource(_s); }} catch(e) {{}}
            }}
            try {{ if (map.getLayer('playback-marker'))  map.removeLayer('playback-marker');  }} catch(e) {{}}
            try {{ if (map.getSource('playback-marker'))      map.removeSource('playback-marker');      }} catch(e) {{}}
            {init}
            // single-circle marker: white ring -> MAC-colored center
            map.addSource('playback-marker',{{type:'geojson',data:{{type:'Feature',geometry:{{type:'Point',coordinates:{first}}}}}}});
            map.addLayer({{id:'playback-marker',type:'circle',source:'playback-marker',
                paint:{{'circle-radius':9,'circle-color':'#44aaff',
                        'circle-stroke-width':3,'circle-stroke-color':'#ffffff','circle-opacity':1}}}});
            window._livePlaybackColorCount = {color_count};
            window._liveAutoFitted = false;
        }}
        window._liveColorIdx = _newIdx;
        var _colorIdx        = _newIdx;
        window._livePb       = [{coords}];
        window._livePbColors = [{c_json}];
        // reinstall frame-update so it always closes over the current _colorIdx
        window._liveUpdatePlayback = function(lon,lat,idx,heading) {{
            var pb        = window._livePb;
            var pb_colors = window._livePbColors;
            var _prev     = window._liveLast;
            window._liveLast = {{lon:lon, lat:lat, hdg:heading||0}};

            var activeColor = (idx < pb_colors.length ? pb_colors[idx] : null)
                            || (pb_colors.length > 0 ? pb_colors[pb_colors.length-1] : '#44aaff');

            // committed history -> all points strictly before the current position
            // the live segment (last committed point -> [lon,lat]) is rendered
            // frame-by-frame so the trajectory never leads the marker.
            var baseByColor = {{}};
            for (var j = 0; j < idx && j < pb.length; j++) {{
                var c = pb_colors[j];
                if (!baseByColor[c]) baseByColor[c] = [];
                baseByColor[c].push(pb[j]);
            }}
            var activeBase = baseByColor[activeColor] || [];

            // finalize all non-active-color sources immediately (unchanged this frame)
            for (var col in _colorIdx) {{
                if (col === activeColor) continue;
                var li   = _colorIdx[col];
                var pts2 = baseByColor[col] || [];
                var c2   = pts2.length >= 2 ? pts2 : pts2.length === 1 ? [pts2[0],pts2[0]] : [[lon,lat],[lon,lat]];
                var src  = map.getSource('playback-traveled-' + li);
                if (src) src.setData({{type:'Feature',geometry:{{type:'LineString',coordinates:c2}}}});
            }}

            // extend the active-color trajectory to a given tip position
            // called once per animation frame to keep the line in sync with the marker
            function updateActiveTraj(tipLon, tipLat) {{
                var li  = _colorIdx[activeColor];
                if (li === undefined) return;
                var src = map.getSource('playback-traveled-' + li);
                if (!src) return;
                var coords = activeBase.concat([[tipLon,tipLat]]);
                if (coords.length < 2) coords = [[tipLon,tipLat],[tipLon,tipLat]];
                src.setData({{type:'Feature',geometry:{{type:'LineString',coordinates:coords}}}});
            }}
            // exposed so the 'move' handler can drive the tip during easeTo
            window._liveUpdateTip = updateActiveTraj;

            var _now = performance.now();
            var _dt  = window._liveTick ? (_now - window._liveTick) : 9999;
            window._liveTick = _now;
            var msrc = map.getSource('playback-marker');

            // css center-marker: a DOM element fixed at 50%/50%
            // in AutoCenter mode it replaces the GeoJSON dot so it is immune to camera-animation desynchronisation and never jitters
            var cm = window._citesCM;
            if (cm) {{
                cm.style.display = window._liveAutoCenter ? 'block' : 'none';
                if (window._liveAutoCenter) cm.style.background = activeColor;
            }}

            if (window._liveAutoCenter) {{
                // camera follows vehicle; GeoJSON marker hidden
                try {{
                    map.setPaintProperty('playback-marker', 'circle-opacity',        0);
                    map.setPaintProperty('playback-marker', 'circle-stroke-opacity', 0);
                }} catch(e) {{}}
                if (msrc) {{
                    msrc.setData({{type:'Feature',geometry:{{type:'Point',coordinates:[lon,lat]}}}});
                    // Tip = current camera center (== CSS marker on screen).
                    // The 'move' handler keeps it pinned during the easeTo animation.
                    var _c0 = map.getCenter();
                    updateActiveTraj(_c0.lng, _c0.lat);
                }}
                // ema on the raw heading, independent of camera bearing
                // using camera bearing as the filter base causes inconsistent damping and impulse catch-up; a dedicated state variable does not.
                if (window._liveSmoothHdg === undefined || !_prev) {{
                    window._liveSmoothHdg = heading || 0;
                }} else {{
                    var _hd = ((heading - window._liveSmoothHdg + 540) % 360) - 180;
                    window._liveSmoothHdg += _hd * 0.1;
                }}
                var cur  = map.getBearing();
                var diff = ((window._liveSmoothHdg - cur + 540) % 360) - 180;
                map.easeTo({{
                    center:   [lon, lat],
                    bearing:  cur + diff,
                    duration: 200,
                    easing:   function(t) {{ return 1 - Math.pow(1 - t, 3); }}
                }});
            }} else if (msrc) {{
                // free-camera: GeoJSON marker visible and moves smoothly across static map
                try {{
                    map.setPaintProperty('playback-marker', 'circle-opacity',        1);
                    map.setPaintProperty('playback-marker', 'circle-stroke-opacity', 1);
                    map.setPaintProperty('playback-marker', 'circle-color', activeColor);
                }} catch(e) {{}}
                if (window._liveSmooth && _prev) {{
                    var fLon = _prev.lon, fLat = _prev.lat;
                    var aniId = window._liveAnimId = (window._liveAnimId || 0) + 1;
                    var dur2 = Math.min(_dt * 1.1, 600);
                    var t0   = _now;
                    (function(thisId) {{
                        function step(t) {{
                            if (window._liveAnimId !== thisId) return;
                            var p = Math.min((t - t0) / dur2, 1.0);
                            var s = p * p * (3 - 2 * p);
                            var iLon = fLon + (lon - fLon) * s;
                            var iLat = fLat + (lat - fLat) * s;
                            if (msrc) msrc.setData({{type:'Feature',geometry:{{type:'Point',
                                coordinates:[iLon,iLat]}}}});
                            updateActiveTraj(iLon, iLat);
                            if (p < 1.0) requestAnimationFrame(step);
                        }}
                        requestAnimationFrame(step);
                    }})(aniId);
                }} else {{
                    msrc.setData({{type:'Feature',geometry:{{type:'Point',coordinates:[lon,lat]}}}});
                    updateActiveTraj(lon, lat);
                }}
            }}
        }};
        if (!window._liveAutoFitted && window._livePb.length > 0) {{
            window._liveAutoFitted = true;
            map.flyTo({{center:window._livePb[window._livePb.length-1],zoom:Math.max(map.getZoom(),15),duration:400}});
        }}
        // Seed the marker only on vehicle / color-structure rebuild.
        // Routine data updates leave the marker to the render-tick driven RAF loop, which otherwise causes a forward/back jitter against applyScene
        if (_rebuild && window._livePb.length > 0) {{
            var _pb = window._livePb;
            window._liveUpdatePlayback(_pb[_pb.length-1][0],_pb[_pb.length-1][1],_pb.length-1,0);
        }}"#)
        }
    };

    format!(r#"(function() {{
    var trajs = {traj_json};

    function applyScene(trajs) {{
        var map = window._liveMap;
        if (!map) return;
        var style = map.getStyle();

        // upsert offline trajectory lines: update source data if it exists, create if new, remove if its MAC is no longer in the current set.
        var activeSids = {{}};
        var allCoords  = [];
        trajs.forEach(function(t) {{
            if (!t.points || t.points.length < 2) return;
            var sid = 'live-traj-' + t.mac.replace(/[^a-zA-Z0-9_-]/g, '_');
            activeSids[sid] = true;
            allCoords = allCoords.concat(t.points);
            var geo = {{type:'Feature',properties:{{}},geometry:{{type:'LineString',coordinates:t.points}}}};
            var src = map.getSource(sid);
            if (src) {{
                src.setData(geo);
                var hitSrc = map.getSource(sid + '-hit');
                if (hitSrc) hitSrc.setData(geo);
            }} else {{
                map.addSource(sid, {{type:'geojson', data:geo}});
                map.addLayer({{id:sid, type:'line', source:sid,
                    layout:{{'line-join':'round','line-cap':'round'}},
                    paint:{{'line-color':t.color,'line-width':3,'line-opacity':{opacity}}}}});
                // invisible {TRAJ_HIT_WIDTH}px hit-target layer on top of the 3px visible
                // line; improves click ergonomics without affecting appearance
                // on click, sets window._citesSeekClick for the rust seek coroutine
                var hitId = sid + '-hit';
                map.addSource(hitId, {{type:'geojson', data:geo}});
                map.addLayer({{id:hitId, type:'line', source:hitId,
                    layout:{{'line-join':'round','line-cap':'round'}},
                    paint:{{'line-color':'#000000','line-width':{TRAJ_HIT_WIDTH},'line-opacity':0}}}});
                map.on('click', hitId, function(e) {{
                    window._citesSeekClick = [e.lngLat.lng, e.lngLat.lat];
                }});
                map.on('mouseenter', hitId, function() {{ map.getCanvas().style.cursor = 'pointer'; }});
                map.on('mouseleave', hitId, function() {{ map.getCanvas().style.cursor = '';         }});
            }}
        }});
        if (style && style.layers)
            style.layers.filter(function(l) {{
                if (l.id.indexOf('live-traj-') !== 0) return false;
                // match both the visible layer and its -hit companion
                var base = l.id.endsWith('-hit') ? l.id.slice(0, -4) : l.id;
                return !activeSids[base];
            }}).forEach(function(l) {{ try {{ map.removeLayer(l.id); }} catch(e) {{}} }});
        if (style && style.sources)
            Object.keys(style.sources).filter(function(s) {{
                if (s.indexOf('live-traj-') !== 0) return false;
                var base = s.endsWith('-hit') ? s.slice(0, -4) : s;
                return !activeSids[base];
            }}).forEach(function(s) {{ try {{ map.removeSource(s); }} catch(e) {{}} }});

        // playback overlay (see pb_block for rebuild logic)
        {pb_block}

        // fit to static trajectory extent (offline mode only; online has no static trajs)
        if (allCoords.length > 1) {{
            var b = allCoords.reduce(function(b,c) {{ return b.extend(c); }},
                       new maplibregl.LngLatBounds(allCoords[0], allCoords[0]));
            map.fitBounds(b, {{padding:40, maxZoom:17}});
        }}
    }}

    window._applyScene = applyScene;

    var map = window._liveMap;
    if (!map || !map.isStyleLoaded()) {{
        window._pendingScene = trajs;
    }} else {{
        applyScene(trajs);
    }}
}})();"#)
}

// component

#[derive(Props, Clone, PartialEq)]
pub struct MapViewProps {
    #[props(default)]
    pub trajectories: Vec<LiveTrajData>,
    /// Non-empty = playback overlay is active (dim traj + show marker + traveled line).
    #[props(default)]
    pub playback_map_pts: Vec<(f64, f64)>,
    /// Per-point color (parallel to `playback_map_pts`), matching each point's MAC trajectory color.
    #[props(default)]
    pub playback_colors: Vec<String>,
}

#[component]
pub fn MapView(props: MapViewProps) -> Element {
    let conn = use_context::<ConnectionService>();
    let pb_active = !props.playback_map_pts.is_empty();

    // effect 1: map initialisation
    let cty = find_country(&conn.country_code.read());
    let init_js  = make_live_map_js(
        &conn.live_tile_server_url.read(), cty.lon, cty.lat, cty.zoom,
    );
    let mut init_sig = use_signal(|| init_js.clone());
    if *init_sig.read() != init_js { init_sig.set(init_js); }
    use_effect(move || {
        let js = init_sig.read().clone();
        spawn(async move { let _ = document::eval(&js).await; });
    });

    // effect 2: scene (trajectory + playback) — rebuilt togehter so layer order stays correct
    let pb_pts_ref: Option<&[(f64,f64)]> = if pb_active {
        Some(&props.playback_map_pts)
    } else {
        None
    };
    let scene_js = make_scene_js(&props.trajectories, pb_pts_ref, &props.playback_colors);
    let mut scene_sig = use_signal(|| scene_js.clone());
    if *scene_sig.read() != scene_js { scene_sig.set(scene_js); }
    use_effect(move || {
        let js = scene_sig.read().clone();
        spawn(async move { let _ = document::eval(&js).await; });
    });

    rsx! {
        div { id: "live-map-container", class: "live-map" }
    }
}
