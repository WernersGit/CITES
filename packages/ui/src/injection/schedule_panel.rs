use dioxus::prelude::*;
use core_logic::config::{InjectionSchedule, RepeatMode};

/// Schedule panel: repeat mode, delayes, timing options, dry-run, and TX power.
///
/// `pres_timing` is passed in as its own signal so the persistence layer
/// in `ConnectionService` can be the single source of truth and survive
/// navigation. The composed `InjectionSchedule` is built at `on_start` time.
#[component]
pub fn SchedulePanel(
    mut schedule: Signal<InjectionSchedule>,
    mut dry_run: Signal<bool>,
    mut tx_power: Signal<Option<u8>>,
    mut pres_timing: Signal<bool>,
) -> Element {
    let repeat_key = use_memo(move || match schedule.read().repeat {
        RepeatMode::Once     => "once",
        RepeatMode::Count(_) => "count",
        RepeatMode::Infinite => "infinite",
    });

    let is_count    = move || matches!(schedule.read().repeat, RepeatMode::Count(_));
    let is_repeated = move || !matches!(schedule.read().repeat, RepeatMode::Once);
    let preserve    = move || *pres_timing.read();

    rsx! {
        div { class: "config-sub-panel",
            h3 { class: "section-title", "Schedule" }

            // Repeat mode
            div { class: "config-section",
                label { class: "config-label sm", "Repeat Mode" }
                select {
                    class: "form-select",
                    value: repeat_key.read().to_string(),
                    onchange: move |e| {
                        let prev_n = match schedule.read().repeat {
                            RepeatMode::Count(n) => n,
                            _ => 3,
                        };
                        schedule.write().repeat = match e.value().as_str() {
                            "count" => RepeatMode::Count(prev_n),
                            "infinite" => RepeatMode::Infinite,
                            _ => RepeatMode::Once,
                        };
                    },
                    option { value: "once", "Once" }
                    option { value: "count", "N Times" }
                    option { value: "infinite", "Infinite" }
                }
            }

            // count input (n times only)
            if is_count() {
                div { class: "config-section",
                    label { class: "config-label sm", "Repeat Count" }
                    input {
                        r#type: "number",
                        min: "1",
                        class: "form-input config-input-narrow",
                        value: match schedule.read().repeat {
                            RepeatMode::Count(n) => n.to_string(),
                            _ => "3".to_string(),
                        },
                        oninput: move |e| {
                            let n = e.value().trim().parse::<u32>().unwrap_or(1).max(1);
                            schedule.write().repeat = RepeatMode::Count(n);
                        },
                    }
                }
            }

            // packet delay
            div { class: "config-section",
                label { class: "config-label sm", "Packet Delay (ms)" }
                p { class: "config-hint",
                    "Pause after each packet. 0 = maximum throughput. \
                     Overridden when Preserve Timing is active."
                }
                input {
                    r#type: "number",
                    min: "0",
                    class: "form-input config-input-narrow",
                    value: schedule.read().packet_delay_ms.to_string(),
                    disabled: preserve(),
                    oninput: move |e| {
                        schedule.write().packet_delay_ms = e.value().trim().parse::<u64>().unwrap_or(0);
                    },
                }
            }

            // loop delay (repeatd modes only)
            if is_repeated() {
                div { class: "config-section",
                    label { class: "config-label sm", "Loop Delay (ms)" }
                    p { class: "config-hint", "Pause between repetitions." }
                    input {
                        r#type: "number",
                        min: "0",
                        class: "form-input config-input-narrow",
                        value: schedule.read().loop_delay_ms.to_string(),
                        oninput: move |e| {
                            schedule.write().loop_delay_ms =
                                e.value().trim().parse::<u64>().unwrap_or(1000);
                        },
                    }
                }
            }

            // preserve timing toggle
            div { class: "toggle-row",
                span { class: "toggle-label", "Preserve Timing" }
                div {
                    class: "toggle-switch",
                    onclick: move |_| pres_timing.with_mut(|v| *v = !*v),
                    div { class: if preserve() { "toggle-track on" } else { "toggle-track off" } }
                    div { class: if preserve() { "toggle-thumb on" } else { "toggle-thumb off" } }
                }
                span { class: "toggle-state-label",
                    if preserve() {
                        "On"
                    } else {
                        "Off"
                    }
                }
            }
            if preserve() {
                p { class: "config-hint", style: "margin: -0.5rem 0 0.75rem;",
                    "Replays original inter-packet gaps. Overrides Packet Delay."
                }
            }

            // jitter
            div { class: "config-section",
                label { class: "config-label sm", "Timing Jitter (ms)" }
                p { class: "config-hint", "Adds 0..=N ms of per-packet variation." }
                input {
                    r#type: "number",
                    min: "0",
                    class: "form-input config-input-narrow",
                    value: schedule.read().jitter_ms.to_string(),
                    oninput: move |e| {
                        schedule.write().jitter_ms = e.value().trim().parse::<u64>().unwrap_or(0);
                    },
                }
            }

            // dry-run toggle
            div { class: "toggle-row",
                span { class: "toggle-label", "Dry Run" }
                div {
                    class: "toggle-switch",
                    onclick: move |_| {
                        let v = !*dry_run.read();
                        dry_run.set(v);
                    },
                    div { class: if *dry_run.read() { "toggle-track on" } else { "toggle-track off" } }
                    div { class: if *dry_run.read() { "toggle-thumb on" } else { "toggle-thumb off" } }
                }
                span { class: "toggle-state-label",
                    if *dry_run.read() {
                        "On (counting only)"
                    } else {
                        "Off"
                    }
                }
            }

            // tx power toggle + input
            div { class: "toggle-row",
                span { class: "toggle-label", "Custom TX Power" }
                div {
                    class: "toggle-switch",
                    onclick: move |_| {
                        tx_power.set(if tx_power.read().is_some() { None } else { Some(20) });
                    },
                    div { class: if tx_power.read().is_some() { "toggle-track on" } else { "toggle-track off" } }
                    div { class: if tx_power.read().is_some() { "toggle-thumb on" } else { "toggle-thumb off" } }
                }
                span { class: "toggle-state-label",
                    if tx_power.read().is_some() {
                        "On"
                    } else {
                        "Off (default)"
                    }
                }
            }
            if tx_power.read().is_some() {
                div { class: "config-section",
                    label { class: "config-label sm", "TX Power (dBm)" }
                    p { class: "config-hint", "Sendeleistung in dBm. Hardware-Maximum: +27 dBm." }
                    input {
                        r#type: "number",
                        min: "1",
                        max: "27",
                        step: "1",
                        class: "form-input config-input-narrow",
                        value: tx_power.read().unwrap_or(20).to_string(),
                        oninput: move |e| {
                            let v = e.value().trim().parse::<u8>().unwrap_or(20).clamp(1, 27);
                            tx_power.set(Some(v));
                        },
                    }
                }
            }
        }
    }
}
