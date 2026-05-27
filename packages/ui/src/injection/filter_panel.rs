use dioxus::prelude::*;
use core_logic::config::{InjectionFilter, ReplayProtocol};

/// Collapsible filter panel for protocol, vehicle-ID, and time-range constrants.
///
/// Writes directly into the caller-owned `Signal<InjectionFilter>` so the
/// parent's `use_memo` reacts without additional callbacks.
#[component]
pub fn FilterPanel(mut filter: Signal<InjectionFilter>) -> Element {
    rsx! {
        div { class: "config-sub-panel",
            h3 { class: "section-title", "Packet Filter" }

            // Protocol filter
            div { class: "config-section",
                label { class: "config-label sm", "Protocols" }
                p { class: "config-hint", "Empty = all protocols." }
                div { class: "checkbox-group",
                    for &proto in ReplayProtocol::ALL {
                        {
                            let checked = filter.read().protocols.contains(&proto);
                            rsx! {
                                label { class: "checkbox-label",
                                    key: "{proto.label()}",
                                    input {
                                        r#type: "checkbox",
                                        checked,
                                        onchange: move |e| {
                                            let mut f = filter.write();
                                            if e.checked() {
                                                if !f.protocols.contains(&proto) {
                                                    f.protocols.push(proto);
                                                }
                                            } else {
                                                f.protocols.retain(|p| *p != proto);
                                            }
                                        },
                                    }
                                    "{proto.label()}"
                                }
                            }
                        }
                    }
                }
            }

            // Vehicle ID filter
            div { class: "config-section",
                label { class: "config-label sm", "Vehicle ID" }
                p { class: "config-hint",
                    "Station ID derived from source MAC (bytes 2-5). Leave blank for all."
                }
                input {
                    r#type: "number",
                    min: "0",
                    class: "form-input config-input-narrow",
                    placeholder: "Any",
                    value: filter.read().vehicle_id.map(|v| v.to_string()).unwrap_or_default(),
                    oninput: move |e| {
                        filter.write().vehicle_id = e.value().trim().parse::<u32>().ok();
                    },
                }
            }

            // Time-range filter
            div { class: "config-section",
                label { class: "config-label sm", "Time Range (ms offset from first packet)" }
                div { class: "injection-range-row",
                    input {
                        r#type: "number",
                        min: "0",
                        class: "form-input",
                        style: "width: 110px;",
                        placeholder: "Start",
                        value: filter.read().time_range_start_ms.map(|v| v.to_string()).unwrap_or_default(),
                        oninput: move |e| {
                            filter.write().time_range_start_ms = e.value().trim().parse::<u64>().ok();
                        },
                    }
                    span { class: "injection-range-sep", "to" }
                    input {
                        r#type: "number",
                        min: "0",
                        class: "form-input",
                        style: "width: 110px;",
                        placeholder: "End",
                        value: filter.read().time_range_end_ms.map(|v| v.to_string()).unwrap_or_default(),
                        oninput: move |e| {
                            filter.write().time_range_end_ms = e.value().trim().parse::<u64>().ok();
                        },
                    }
                }
            }

            // Clear button
            button {
                class: "btn btn-secondary",
                style: "align-self: flex-start;",
                onclick: move |_| filter.set(InjectionFilter::default()),
                "Clear Filter"
            }
        }
    }
}
