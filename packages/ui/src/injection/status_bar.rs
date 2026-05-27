use dioxus::prelude::*;
use core_logic::config::{InjectionEngineState, InjectionStatus};

/// Control and status bar rendered at the bottom of the injection page.
///
/// The parent owns all state mutations; this component is purely presentational
/// and fires typed event handlers.
#[component]
pub fn StatusBar(
    status:    InjectionStatus,
    can_start: bool,
    on_start:  EventHandler<MouseEvent>,
    on_stop:   EventHandler<MouseEvent>,
    on_pause:  EventHandler<MouseEvent>,
) -> Element {
    let is_running = matches!(status.state, InjectionEngineState::Running);
    let is_paused  = matches!(status.state, InjectionEngineState::Paused);
    let is_active  = is_running || is_paused;
    let show_stats = is_active || matches!(
        status.state,
        InjectionEngineState::Completed | InjectionEngineState::Error(_)
    );

    let progress_pct: u32 = if status.packets_total > 0 {
        ((status.packets_sent as f64 / status.packets_total as f64) * 100.0) as u32
    } else {
        0
    };

    let state_class = match &status.state {
        InjectionEngineState::Running   => "status-dot running",
        InjectionEngineState::Paused    => "status-dot paused",
        InjectionEngineState::Completed => "status-dot completed",
        InjectionEngineState::Error(_)  => "status-dot error",
        InjectionEngineState::Idle      => "status-dot idle",
    };

    rsx! {
        div { class: "injection-status-bar card",
            div { class: "injection-controls-row",
                // Action buttons
                div { class: "injection-btns",
                    if !is_active {
                        button {
                            class: "btn btn-success",
                            disabled: !can_start,
                            onclick: move |e| on_start.call(e),
                            "Start"
                        }
                    } else {
                        button {
                            class: "btn btn-secondary",
                            onclick: move |e| on_pause.call(e),
                            if is_paused { "Resume" } else { "Pause" }
                        }
                        button {
                            class: "btn btn-danger",
                            onclick: move |e| on_stop.call(e),
                            "Stop"
                        }
                    }
                }

                // Status indicators
                div { class: "injection-stats",
                    span { class: "{state_class}" }
                    span { class: "status-label", "{status.state.label()}" }

                    if show_stats {
                        span { class: "stat-item",
                            strong { "{status.packets_sent}" }
                            " / {status.packets_total} pkts"
                        }
                        if status.current_iteration > 0 {
                            span { class: "stat-item",
                                "Iter {status.current_iteration}"
                            }
                        }
                        span { class: "stat-item mono",
                            "{format_elapsed(status.elapsed_ms)}"
                        }
                    }

                    if let InjectionEngineState::Error(ref msg) = status.state {
                        span { class: "injection-error", "{msg}" }
                    }
                }
            }

            // Progress bar (finite runs only)
            if is_active && status.packets_total > 0 {
                div { class: "injection-progress-track",
                    div {
                        class: "injection-progress-fill",
                        style: "width: {progress_pct}%;",
                    }
                }
                p { class: "injection-progress-label",
                    "{progress_pct}% — {status.packets_sent} of {status.packets_total} packets"
                }
            }
        }
    }
}

fn format_elapsed(ms: u64) -> String {
    let total_s = ms / 1000;
    format!("{:02}:{:02}", total_s / 60, total_s % 60)
}
