use dioxus::prelude::*;
use platform::{ConnectionState, ConnectionService};

const HOME_CSS: Asset = asset!("/assets/styling/home.css");

#[component]
pub fn Home() -> Element {
    let connection = use_context::<ConnectionService>();
    let mut hostname = use_signal(String::new);
    let mut ip_error = use_signal(|| Option::<String>::None);
    let state = connection.state.read().clone();

    rsx! {
        document::Link { rel: "stylesheet", href: HOME_CSS }

        div { class: "home-page",
            h1 { class: "page-title", "CITES Gateway" }

            match state {
                ConnectionState::Disconnected => rsx! {
                    div { class: "connect-grid",
                        div { class: "card",
                            h2 { class: "card-title", "Bluetooth Node" }
                            p  { class: "card-desc", "Scan for local CITES edge nodes via BLE." }
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let conn = connection.clone();
                                    move |_| {
                                        let mut c = conn.clone();
                                        spawn(async move { c.start_bluetooth_scan().await; });
                                    }
                                },
                                "Scan for BT Nodes"
                            }
                        }
                        div { class: "card",
                            h2 { class: "card-title", "IP / Hostname Node" }
                            p  { class: "card-desc", "Connect to a Cloud or local API via netwok." }
                            input {
                                class: "form-input hostname-input",
                                placeholder: "e.g. 192.168.1.100",
                                value: "{hostname}",
                                oninput: move |e| {
                                    *hostname.write() = e.value();
                                    ip_error.set(None);
                                },
                            }
                            button {
                                class: "btn btn-secondary",
                                disabled: hostname.read().is_empty(),
                                onclick: {
                                    let conn = connection.clone();
                                    move |_| {
                                        let ip = hostname.read().clone();
                                        let mut c = conn.clone();
                                        ip_error.set(None);
                                        spawn(async move {
                                            if let Err(e) = c.connect_ip(&ip).await {
                                                ip_error.set(Some(e));
                                            }
                                        });
                                    }
                                },
                                "Connect via IP"
                            }
                            if let Some(err) = ip_error.read().clone() {
                                p { class: "status-error", "{err}" }
                            }
                        }
                    }
                },

                ConnectionState::Scanning => rsx! {
                    div { class: "card",
                        h2 { class: "status-scanning", "Scanning..." }
                        p  { class: "card-desc", "Searching for CITES BT nodes nearby..." }
                        div { class: "spinner" }
                    }
                },

                ConnectionState::DevicesFound(devices) => rsx! {
                    div { class: "card",
                        h2 { class: "card-title", "Found BT Nodes" }
                        if devices.is_empty() {
                            p { class: "status-error", "No CITES nodes discovered." }
                            button {
                                class: "btn btn-secondary",
                                onclick: {
                                    let mut conn = connection.clone();
                                    move |_| conn.disconnect()
                                },
                                "Back"
                            }
                        } else {
                            ul { class: "device-list",
                                for target_dev in devices.clone() {
                                    li { class: "device-item",
                                        span { class: "device-name", "{target_dev.name}" }
                                        button {
                                            class: "btn btn-success",
                                            onclick: {
                                                let id_clone   = target_dev.id.clone();
                                                let name_clone = target_dev.name.clone();
                                                let conn = connection.clone();
                                                move |_| {
                                                    let ic = id_clone.clone();
                                                    let nc = name_clone.clone();
                                                    let mut c = conn.clone();
                                                    spawn(async move { c.connect_to_device(ic, nc).await; });
                                                }
                                            },
                                            "Connect"
                                        }
                                    }
                                }
                            }
                            button {
                                class: "btn btn-secondary",
                                onclick: {
                                    let mut conn = connection.clone();
                                    move |_| conn.disconnect()
                                },
                                "Cancel"
                            }
                        }
                    }
                },

                ConnectionState::Connecting(info) => rsx! {
                    div { class: "card",
                        p { class: "status-scanning", "Connecting to {info}..." }
                    }
                },

                ConnectionState::ConnectedBT(info) | ConnectionState::ConnectedIP(info) => {
                    let stats = connection.pcap_stats.read();
                    rsx! {
                        div { class: "card",
                            h2 { class: "status-connected", "Connected to {info}" }
                            div { class: "stat-grid",
                                div { class: "stat-cell",
                                    span { class: "stat-label", "Packets Captured" }
                                    div  { class: "stat-value", "{stats.total_packets}" }
                                }
                                div { class: "stat-cell",
                                    span { class: "stat-label", "Data Transferred" }
                                    div  { class: "stat-value", "{stats.total_bytes} B" }
                                }
                                div { class: "stat-cell full-width warning",
                                    span { class: "stat-label warning", "Dropped Fragments (BLE Loss)" }
                                    div  { class: "stat-value warning", "{stats.dropped_fragments}" }
                                }
                            }
                            button {
                                class: "btn btn-danger btn-full",
                                onclick: {
                                    let mut conn = connection.clone();
                                    move |_| conn.disconnect()
                                },
                                "Disconnect"
                            }
                        }
                    }
                }
            }
        }
    }
}
