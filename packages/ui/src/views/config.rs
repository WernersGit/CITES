use dioxus::prelude::*;
use futures::StreamExt;
use core_logic::config::{RepeatModeConfig, ReplayProtocol, TrackingWarningConfig, NodeConfig, LogLevel};
use platform::ConnectionService;
use crate::countries::COUNTRIES;

const CONFIG_CSS: Asset = asset!("/assets/styling/config.css");

/// Top-level configuration view.
///
/// `available_vehicle_ids` — virtual vehicle IDs currently seen in received
/// traffic; passed to the replay vehicle filter so the dropdown stays current.
#[component]
pub fn ConfigView(#[props(default)] available_vehicle_ids: Vec<u32>) -> Element {
    let mut repeat_cfg   = use_signal(RepeatModeConfig::default);
    let mut conn_svc     = use_context::<ConnectionService>();
    let mut warn_cfg = conn_svc.tracking_warning_cfg;
    let mut node_cfg     = conn_svc.node_cfg;
    let mut tile_url = conn_svc.live_tile_server_url;
    let mut country  = conn_svc.country_code;
    let mut replay_count  = use_signal(|| 0u64);
    let mut unix_ts = conn_svc.ts_unix_format;
    let mut ts_gnw     = conn_svc.ts_use_gnw;

    // restore saved RepeatModeConfig from localStorage on mount
    use_effect(move || {
        spawn(async move {
            let key = crate::REPLAY_CONFIG_KEY;
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{key}') || '')")
            );
            if let Ok(json) = eval.recv::<String>().await {
                if let Ok(cfg) = serde_json::from_str::<RepeatModeConfig>(&json) {
                    repeat_cfg.set(cfg);
                }
            }
        });
    });

    //TODO: make debounce delay configurable
    let handle = use_coroutine(move |mut rx: UnboundedReceiver<RepeatModeConfig>| async move {
        loop {
            let Some(mut latest) = rx.next().await else { break };
            loop {
                match async_std::future::timeout(
                    std::time::Duration::from_millis(500),
                    rx.next(),
                ).await {
                    Ok(Some(newer)) => latest = newer,
                    Ok(None) => { let _ = conn_svc.push_replay_config(latest).await; return; }
                    Err(_) => break,
                }
            }
            let _ = conn_svc.push_replay_config(latest).await;
        }
    });

    // polls replay counter every 2 s while replay is active
    use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            async_std::task::sleep(std::time::Duration::from_secs(2)).await;
            if repeat_cfg.read().enabled {
                if let Ok(status) = conn_svc.fetch_status().await {
                    replay_count.set(status.replay_count);
                }
            }
        }
    });

    rsx! {
        document::Link { rel: "stylesheet", href: CONFIG_CSS }

        div { class: "config-page",
            h2 { class: "page-title", "Configuration" }

            // map
            div { class: "config-group",
                h3 { class: "config-group-title", "Map" }

                div { class: "config-section",
                    label { class: "config-label", "Tile Server" }
                    p { class: "config-hint",
                        "Base URL of the local tileserver-gl instance used by all map veiws \
                         (Live and Analysis). A trailing slash is stripped on blur. \
                         Persisted across sessions."
                    }
                    input {
                        r#type: "url",
                        class: "form-input config-input",
                        value: tile_url.read().clone(),
                        oninput: move |e| tile_url.set(e.value()),
                        onblur: move |_| {
                            let url = tile_url.read().trim_end_matches('/').to_string();
                            tile_url.set(url.clone());
                            let js = crate::save_live_tile_url(&url);
                            spawn(async move {
                                let _ = document::eval(&js).await;
                            });
                        },
                    }
                }

                div { class: "config-section",
                    label { class: "config-label", "Home Country" }
                    p { class: "config-hint",
                        "Sets the initial map center and zoom level for both Live and Analysis maps."
                    }
                    select {
                        class: "form-select config-select",
                        value: country.read().clone(),
                        onchange: move |e| {
                            let cc = e.value();
                            country.set(cc.clone());
                            let js = crate::save_country_code(&cc);
                            spawn(async move {
                                let _ = document::eval(&js).await;
                            });
                        },
                        for c in COUNTRIES.iter() {
                            option {
                                value: "{c.code}",
                                selected: *country.read() == c.code,
                                "{c.name}"
                            }
                        }
                    }
                }
            }

            // node
            div { class: "config-group",
                h3 { class: "config-group-title", "Node" }

                div { class: "config-section",
                    label { class: "config-label", "System Log Level" }
                    p { class: "config-hint",
                        "Applies to both client and the connected node immediatly. \
                         Persisted across sessions and pushed on every future connection."
                    }
                    select {
                        class: "form-select config-select",
                        value: node_cfg.read().log_level.as_filter_str(),
                        onchange: move |e| {
                            let lvl = LogLevel::from_filter_str(&e.value())
                                .unwrap_or(LogLevel::Info);
                            node_cfg.write().log_level = lvl;
                            let cfg = node_cfg.read().clone();
                            let js = crate::save_node_config(&cfg);
                            spawn(async move {
                                let _ = document::eval(&js).await;
                            });
                            spawn(async move {
                                let _ = conn_svc.push_node_config(cfg).await;
                            });
                        },
                        for & level in LogLevel::ALL {
                            option {
                                value: "{level.as_filter_str()}",
                                selected: node_cfg.read().log_level == level,
                                "{level.label()}"
                            }
                        }
                    }
                }

                NodeSettingsPanel { node_cfg }
            }

            // replay
            div { class: "config-group",
                h3 { class: "config-group-title", "Replay" }
                ToggleRow {
                    label: "Replay Mode",
                    enabled: repeat_cfg.read().enabled,
                    on_toggle: move |_| {
                        repeat_cfg.with_mut(|c| c.enabled = !c.enabled);
                        handle.send(repeat_cfg.read().clone());
                        let js = crate::save_replay_config(&repeat_cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
                if repeat_cfg.read().enabled {
                    ReplayPanel {
                        cfg: repeat_cfg,
                        apply_handle: handle,
                        available_vehicle_ids,
                        replay_count,
                    }
                }
            }

            // tracking warning
            div { class: "config-group",
                h3 { class: "config-group-title", "Tracking Warning" }
                ToggleRow {
                    label: "Tracking Warning",
                    enabled: warn_cfg.read().enabled,
                    on_toggle: move |_| {
                        warn_cfg.with_mut(|c| c.enabled = !c.enabled);
                        let js = crate::save_tracking_warning_config(&warn_cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
                if warn_cfg.read().enabled {
                    TrackingWarningPanel { cfg: warn_cfg }
                }
            }

            // tts announcements
            div { class: "config-group",
                h3 { class: "config-group-title", "TTS Announcements" }
                ToggleRow {
                    label: "TTS Announcements",
                    enabled: *conn_svc.tts_enabled.read(),
                    on_toggle: move |_| {
                        let new_val = !*conn_svc.tts_enabled.read();
                        conn_svc.tts_enabled.set(new_val);
                        let js = crate::save_tts_enabled(new_val);
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
                if *conn_svc.tts_enabled.read() {
                    TtsPanel {}
                }
            }

            // display
            div { class: "config-group",
                h3 { class: "config-group-title", "Display" }

                div { class: "config-section",
                    label { class: "config-label", "Packet Timestamp (Offline)" }
                    p { class: "config-hint",
                        "Controls the timestmap shown next to the TRAJECTORY heading in offline mode."
                    }
                }
                ToggleRow {
                    label: "Unix Timestamp",
                    enabled: *unix_ts.read(),
                    on_toggle: move |_| {
                        let new_val = !*unix_ts.read();
                        unix_ts.set(new_val);
                        let js = crate::save_ts_unix_format(new_val);
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
                div { class: "config-hint config-hint-indent",
                    if *unix_ts.read() {
                        "Format: Unix ms (e.g. 1746000000000)"
                    } else {
                        "Format: hh:mm:ss dd.mm.yyyy"
                    }
                }
                ToggleRow {
                    label: "GNW Packet Timestamp",
                    enabled: *ts_gnw.read(),
                    on_toggle: move |_| {
                        let new_val = !*ts_gnw.read();
                        ts_gnw.set(new_val);
                        let js = crate::save_ts_use_gnw(new_val);
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
                div { class: "config-hint config-hint-indent",
                    if *ts_gnw.read() {
                        "Source: GeoNetworking LPV TST (EN 302 636-4-1)"
                    } else {
                        "Source: PCAP capture timestamp"
                    }
                }
            }
        }
    }
}

// shared toggle row

/// A labelled on/off toggle switch row.  The caller owns state mutation via
/// `on_toggle`; this component is purely presentational.
#[component]
fn ToggleRow(
    label: &'static str,
    enabled: bool,
    on_toggle: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div { class: "toggle-row",
            span { class: "toggle-label", "{label}" }
            div { class: "toggle-switch", onclick: move |e| on_toggle.call(e),
                div { class: if enabled { "toggle-track on" } else { "toggle-track off" } }
                div { class: if enabled { "toggle-thumb on" } else { "toggle-thumb off" } }
            }
            span { class: "toggle-state-label",
                if enabled {
                    "Active"
                } else {
                    "Inactive"
                }
            }
        }
    }
}

// replay panel

#[component]
fn ReplayPanel(
    cfg: Signal<RepeatModeConfig>,
    apply_handle: Coroutine<RepeatModeConfig>,
    available_vehicle_ids: Vec<u32>,
    replay_count: Signal<u64>,
) -> Element {
    rsx! {
        div { class: "config-sub-panel",

            // vehicle filter
            div { class: "config-section",
                label { class: "config-label sm", "Vehicle Filter" }
                p { class: "config-hint",
                    "Only messages from the selected vehicle will be replayed. \
                     Select \"All Vehicles\" to replay all."
                }
                select {
                    class: "form-select config-select",
                    value: match cfg.read().vehicle_id_filter {
                        None => "all".to_string(),
                        Some(id) => id.to_string(),
                    },
                    onchange: move |e| {
                        cfg.write().vehicle_id_filter = match e.value().as_str() {
                            "all" => None,
                            s => s.parse::<u32>().ok(),
                        };
                        apply_handle.send(cfg.read().clone());
                        let js = crate::save_replay_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                    option { value: "all", "All Vehicles" }
                    for id in available_vehicle_ids.iter().copied() {
                        option { value: "{id}", "Vehicle {id}" }
                    }
                }
            }

            // protocol filter
            div { class: "config-section",
                label { class: "config-label sm", "Protocol Filter" }
                p { class: "config-hint",
                    "Select messsage types to replay. Deselecting all equals selecting all."
                }
                div { class: "checkbox-group",
                    for & proto in ReplayProtocol::ALL {
                        {
                            let is_checked = cfg.read().protocol_filter.contains(&proto);
                            rsx! {
                                label { class: "checkbox-label",
                                    input {
                                        r#type: "checkbox",
                                        checked: is_checked,
                                        onchange: move |e| {
                                            let mut c = cfg.write();
                                            if e.checked() {
                                                if !c.protocol_filter.contains(&proto) {
                                                    c.protocol_filter.push(proto);
                                                }
                                            } else {
                                                c.protocol_filter.retain(|p| *p != proto);
                                            }
                                            drop(c);
                                            apply_handle.send(cfg.read().clone());
                                            let js = crate::save_replay_config(&cfg.read());
                                            spawn(async move {
                                                let _ = document::eval(&js).await;
                                            });
                                        },
                                    }
                                    "{proto.label()}"
                                }
                            }
                        }
                    }
                }
            }

            // replay delay
            div { class: "config-section",
                label { class: "config-label sm", "Replay Delay (ms)" }
                p { class: "config-hint",
                    "Pause after each replayed packet. Set to 0 for maximum throughput."
                }
                input {
                    r#type: "number",
                    min: "0",
                    step: "10",
                    class: "form-input config-input-narrow",
                    value: cfg.read().delay_ms.to_string(),
                    onchange: move |e| {
                        let delay = e.value().trim().parse::<u64>().unwrap_or(0);
                        cfg.write().delay_ms = delay;
                        apply_handle.send(cfg.read().clone());
                        let js = crate::save_replay_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
            }

            // tx power toggle + input
            div { class: "toggle-row",
                span { class: "toggle-label", "Custom TX Power" }
                div {
                    class: "toggle-switch",
                    onclick: move |_| {
                        let cur = cfg.read().tx_power_dbm;
                        cfg.write().tx_power_dbm = if cur.is_some() { None } else { Some(20) };
                        apply_handle.send(cfg.read().clone());
                        let js = crate::save_replay_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                    div { class: if cfg.read().tx_power_dbm.is_some() { "toggle-track on" } else { "toggle-track off" } }
                    div { class: if cfg.read().tx_power_dbm.is_some() { "toggle-thumb on" } else { "toggle-thumb off" } }
                }
                span { class: "toggle-state-label",
                    if cfg.read().tx_power_dbm.is_some() {
                        "Aktiv"
                    } else {
                        "Standard"
                    }
                }
            }
            if cfg.read().tx_power_dbm.is_some() {
                div { class: "config-section",
                    label { class: "config-label sm", "TX Power (dBm)" }
                    p { class: "config-hint", "Sendeleistung in dBm. Hardware-Maximum: +27 dBm." }
                    input {
                        r#type: "number",
                        min: "1",
                        max: "27",
                        step: "1",
                        class: "form-input config-input-narrow",
                        value: cfg.read().tx_power_dbm.unwrap_or(20).to_string(),
                        onchange: move |e| {
                            let v = e.value().trim().parse::<u8>().unwrap_or(20).clamp(1, 27);
                            cfg.write().tx_power_dbm = Some(v);
                            apply_handle.send(cfg.read().clone());
                            let js = crate::save_replay_config(&cfg.read());
                            spawn(async move {
                                let _ = document::eval(&js).await;
                            });
                        },
                    }
                }
            }

            // filter summary
            {
                let c = cfg.read();
                let v_desc = match c.vehicle_id_filter {
                    None => "all vehicles".to_string(),
                    Some(id) => format!("vehicle {id}"),
                };
                let proto_desc = if c.protocol_filter.is_empty() {
                    "all protocols".to_string()
                } else {
                    c.protocol_filter.iter().map(|p| p.label()).collect::<Vec<_>>().join(", ")
                };
                rsx! {
                    div { class: "summary-pill",
                        "Replaying "
                        strong { "{proto_desc}" }
                        " from "
                        strong { "{v_desc}" }
                    }
                }
            }

            // replay counter
            div { class: "replay-counter",
                span { class: "replay-counter-label", "Packets replayed:" }
                span { class: "replay-counter-value", "{replay_count}" }
            }
        }
    }
}

#[component]
fn TtsPanel() -> Element {
    let mut conn_svc = use_context::<ConnectionService>();
    let mut timeout_input = use_signal(|| conn_svc.foreign_vehicle_timeout_ms.read().to_string());

    rsx! {
        div { class: "config-sub-panel",

            // language selection
            div { class: "config-section",
                label { class: "config-label sm", "Output Language" }
                p { class: "config-hint",
                    "Selects the TTS voice language. \"System Default\" uses the OS default voice."
                }
                div { class: "tts-lang-row",
                    select {
                        class: "form-select config-select",
                        value: conn_svc.tts_language.read().clone(),
                        onchange: move |e| {
                            let lang = e.value();
                            conn_svc.tts_language.set(lang.clone());
                            let js = crate::save_tts_language(&lang);
                            spawn(async move {
                                let _ = document::eval(&js).await;
                            });
                        },
                        option { value: "", "System Default" }
                        option { value: "de", "Deutsch" }
                        option { value: "en", "English" }
                    }
                    button {
                        class: "btn btn-secondary tts-test-btn",
                        onclick: move |_| {
                            conn_svc.announce(platform::tts::TtsMessage::Test);
                        },
                        "Test"
                    }
                }
            }

            // foreign vehicle timeout
            div { class: "config-section",
                label { class: "config-label sm", "Foreign vehicle timeout (ms)" }
                p { class: "config-hint",
                    "How long a foreign MAC must be silent before \
                     \"Fremdfahrzeug verloren\" is announced."
                }
                input {
                    r#type: "number",
                    min: "100",
                    step: "100",
                    class: "form-input config-input-narrow",
                    value: "{timeout_input}",
                    onchange: move |e| {
                        let ms = e.value().trim().parse::<u64>().unwrap_or(1000).max(100);
                        timeout_input.set(ms.to_string());
                        *conn_svc.foreign_vehicle_timeout_ms.write() = ms;
                    },
                }
            }
        }
    }
}

// node settings panel

#[component]
fn NodeSettingsPanel(node_cfg: Signal<NodeConfig>) -> Element {
    let conn_svc = use_context::<ConnectionService>();
    let mut port_input = use_signal(|| node_cfg.read().api_port.to_string());

    // read the node's actual port once when Config page opens
    // if not connnected, fetch_node_config returns an error (silently ignored)
    use_effect(move || {
        spawn(async move {
            if let Ok(fetched) = conn_svc.fetch_node_config().await {
                port_input.set(fetched.api_port.to_string());
                node_cfg.write().api_port = fetched.api_port;
            }
        });
    });

    rsx! {
        div { class: "config-section",
            label { class: "config-label", "Node API Port" }
            p { class: "config-hint", "Takes effect after restarting the node." }
            input {
                r#type: "number",
                min: "1024",
                max: "65535",
                step: "1",
                class: "form-input config-input-narrow",
                value: "{port_input}",
                onchange: move |e| {
                    let p = e.value().trim().parse::<u16>().unwrap_or(8080).max(1024);
                    port_input.set(p.to_string());
                    node_cfg.write().api_port = p;
                    let cfg = node_cfg.read().clone();
                    let js = crate::save_node_config(&cfg);
                    spawn(async move {
                        let _ = document::eval(&js).await;
                    });
                    spawn(async move {
                        let _ = conn_svc.push_node_config(cfg).await;
                    });
                },
            }
        }
    }
}

// tracking warning panel

#[component]
fn TrackingWarningPanel(cfg: Signal<TrackingWarningConfig>) -> Element {
    rsx! {
        div { class: "config-sub-panel",

            div { class: "config-section",
                label { class: "config-label sm", "Minimum visibility duration (min)" }
                p { class: "config-hint",
                    "Minutes a vehicle must remain continuously visible — gaps within the \
                     tolerance window do not reset the timer — before the warning fires."
                }
                input {
                    r#type: "number",
                    min: "1",
                    step: "1",
                    class: "form-input config-input-narrow",
                    value: cfg.read().min_visible_minutes.to_string(),
                    onchange: move |e| {
                        let v = e.value().trim().parse::<u32>().unwrap_or(5).max(1);
                        cfg.write().min_visible_minutes = v;
                        let js = crate::save_tracking_warning_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
            }

            div { class: "config-section",
                label { class: "config-label sm", "Gap tolerance (s)" }
                p { class: "config-hint",
                    "Seconds a vehicle may be absent between sightings without resetting \
                     the visibility timer."
                }
                input {
                    r#type: "number",
                    min: "0",
                    step: "1",
                    class: "form-input config-input-narrow",
                    value: cfg.read().gap_tolerance_secs.to_string(),
                    onchange: move |e| {
                        let v = e.value().trim().parse::<u32>().unwrap_or(30);
                        cfg.write().gap_tolerance_secs = v;
                        let js = crate::save_tracking_warning_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
            }

            div { class: "config-section",
                label { class: "config-label sm", "Minimum travel distance (km)" }
                p { class: "config-hint",
                    "Kilometres a vehicle must travel continuously before the distance \
                     warning fires. Suited for extra-urban scenarios where the time \
                     threshold would be reached too late."
                }
                input {
                    r#type: "number",
                    min: "0.1",
                    step: "0.1",
                    class: "form-input config-input-narrow",
                    value: format!("{:.1}", cfg.read().min_visible_km),
                    onchange: move |e| {
                        let v = e.value().trim().parse::<f64>().unwrap_or(5.0).max(0.1);
                        cfg.write().min_visible_km = v;
                        let js = crate::save_tracking_warning_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
            }

            div { class: "config-section",
                label { class: "config-label sm", "Distance gap tolerance (km)" }
                p { class: "config-hint",
                    "Maximum straight-line distance between consecutive positions still \
                     considered continuous travel. Resets the distance accumulator when exceeded."
                }
                input {
                    r#type: "number",
                    min: "0",
                    step: "0.1",
                    class: "form-input config-input-narrow",
                    value: format!("{:.1}", cfg.read().gap_tolerance_km),
                    onchange: move |e| {
                        let v = e.value().trim().parse::<f64>().unwrap_or(0.5).max(0.0);
                        cfg.write().gap_tolerance_km = v;
                        let js = crate::save_tracking_warning_config(&cfg.read());
                        spawn(async move {
                            let _ = document::eval(&js).await;
                        });
                    },
                }
            }
        }
    }
}
