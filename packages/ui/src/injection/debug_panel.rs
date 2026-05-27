use dioxus::prelude::*;

/// Advanced/debug switches for the injection run. Kept separate from the
/// main schedule so production users do not flip them by accident.
#[component]
pub fn DebugPanel(mut filter_inj: Signal<bool>) -> Element {
    let on = move || *filter_inj.read();

    rsx! {
        div { class: "config-sub-panel debug-panel",
            h3 { class: "section-title", "Advanced / Debug" }
            p { class: "config-hint",
                "Defaults are safe for production. Only change for development."
            }

            // Filter-injected toggle
            div { class: "toggle-row",
                span { class: "toggle-label", "Filter Injected Packets" }
                div {
                    class: "toggle-switch",
                    onclick: move |_| filter_inj.with_mut(|v| *v = !*v),
                    div { class: if on() { "toggle-track on" } else { "toggle-track off" } }
                    div { class: if on() { "toggle-thumb on" } else { "toggle-thumb off" } }
                }
                span { class: "toggle-state-label",
                    if on() {
                        "On (default)"
                    } else {
                        "Off (debug)"
                    }
                }
            }
            p { class: "config-hint", style: "margin: -0.5rem 0 0.75rem;",
                "When on, the node drops loopback copies of injected frames so the \
                 client sees only over-the-air traffic. Disable to observe the raw \
                 loopback at the client."
            }
        }
    }
}
