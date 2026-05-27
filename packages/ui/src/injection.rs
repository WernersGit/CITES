mod debug_panel;
mod filter_panel;
mod packet_table;
mod schedule_panel;
mod status_bar;

use dioxus::prelude::*;

use core_logic::config::{
    InjectionConfig, InjectionEngineState, InjectionFilter,
    InjectionSchedule, ReplayProtocol,
};
use core_logic::pcap_parser::ParsedPacket;
use platform::{ConnectionService, ConnectionState};

use debug_panel::DebugPanel;
use filter_panel::FilterPanel;
use packet_table::{PacketSummary, PacketTable};
use schedule_panel::SchedulePanel;
use status_bar::StatusBar;

const INJECTION_CSS: Asset = asset!("/assets/styling/injection.css");
const CONFIG_CSS:    Asset = asset!("/assets/styling/config.css");

// main view

/// Injection page: load an archive from the node, configure filters and schedule, then start/stop/pause packet injection.
#[component]
pub fn InjectionView() -> Element {
    let conn = use_context::<ConnectionService>();

    // source
    let mut archive_list = use_signal(Vec::<String>::new);
    // ble: packet counts keyed by filename
    let mut counts: Signal<std::collections::HashMap<String, u64>> = use_signal(std::collections::HashMap::new);
    let mut selected    = use_signal(String::new);
    let mut pkts        = use_signal(Vec::<ParsedPacket>::new);
    let mut load_busy   = use_signal(|| false);
    let mut load_error  = use_signal(|| Option::<String>::None);

    // filter + schedule
    let mut filter   = use_signal(InjectionFilter::default);
    let mut schedule = use_signal(InjectionSchedule::default);
    let mut dry_run  = use_signal(|| false);
    let mut tx_power = use_signal(|| Option::<u8>::None);

    // sticky across navigation; navbar coroutine resets to defaults after run end
    let mut filter_inj   = conn.filter_inj;
    let mut pres_timing  = conn.pres_timing;
    let mut inj_status   = conn.inj_status;
    let mut inj_polling  = conn.inj_polling;
    let mut action_error = use_signal(|| Option::<String>::None);

    let is_ble = matches!(*conn.state.read(), ConnectionState::ConnectedBT(_));

    // live-push the filter toggle to the node while an injection is running
    // The navbar coroutine pulls the server's reported value back into the same signal, so this effect only fires for genuine user clicks (not on echo).
    use_effect(move || {
        let on = *filter_inj.read();
        let is_active = matches!(
            inj_status.read().state,
            InjectionEngineState::Running | InjectionEngineState::Paused
        );
        if is_active && inj_status.read().filter_inj != on {
            spawn(async move { let _ = conn.set_inj_filter(on).await; });
        }
    });

    // fetch archive list (and BLE packet counts) on mount
    use_effect(move || {
        spawn(async move {
            if let Ok(list) = conn.fetch_archive_list().await {
                if let Some(first) = list.first() {
                    selected.set(first.clone());
                }
                archive_list.set(list);
            }
            if let Ok(cnt_data) = conn.fetch_archive_packet_counts().await {
                counts.set(cnt_data.into_iter().collect());
            }
        });
    });

    //TODO: make preview row limt configurable
    let filtered: Memo<Vec<PacketSummary>> = use_memo(move || {
        let pkts    = pkts.read();
        let f       = filter.read();
        let base_ts = pkts.iter().map(|p| p.timestamp_ms).min().unwrap_or(0);
        pkts.iter()
            .filter(|p| matches_filter(p, &f, base_ts))
            .take(200)
            .map(|p| PacketSummary::from_packet(p, base_ts))
            .collect()
    });

    let total: Memo<usize> = use_memo(move || {
        let pkts    = pkts.read();
        let f       = filter.read();
        let base_ts = pkts.iter().map(|p| p.timestamp_ms).min().unwrap_or(0);
        pkts.iter().filter(|p| matches_filter(p, &f, base_ts)).count()
    });

    // action handlers

    let on_refresh = move |_| {
        spawn(async move {
            if let Ok(list) = conn.fetch_archive_list().await {
                archive_list.set(list);
            }
        });
    };

    let on_load = move |_| {
        let fname = selected.read().clone();
        if fname.is_empty() { return; }
        spawn(async move {
            load_busy.set(true);
            load_error.set(None);
            pkts.set(Vec::new());
            match conn.fetch_archive_file(&fname).await {
                Ok(bytes) => match core_logic::pcap_parser::PcapParser::parse_bytes(&bytes) {
                    Ok(parsed) if parsed.is_empty() => load_error.set(Some(
                        "Archive contains 0 parseable Car2X frames. \
                         The file may be empty, still initializing, or use an unsupported link type.".into()
                    )),
                    Ok(parsed) => pkts.set(parsed),
                    Err(e)   => load_error.set(Some(e.to_string())),
                },
                Err(e) => load_error.set(Some(e)),
            }
            load_busy.set(false);
        });
    };

    let on_start = move |_| {
        let fname = selected.read().clone();
        if fname.is_empty() { return; }
        // overlay the sticky preserve-timing signal onto the schedule snapshot
        let mut sched = schedule.read().clone();
        sched.preserve_timing = *pres_timing.read();
        let config = InjectionConfig {
            archive_filename: fname,
            filter:           filter.read().clone(),
            schedule:         sched,
            dry_run:          *dry_run.read(),
            tx_power_dbm:     *tx_power.read(),
            filter_inj:       *filter_inj.read(),
        };
        action_error.set(None);
        spawn(async move {
            match conn.start_injection(config).await {
                Ok(_) => {
                    inj_status.with_mut(|s| s.state = InjectionEngineState::Running);
                    inj_polling.set(true);
                }
                Err(e) => action_error.set(Some(e)),
            }
        });
    };

    let on_stop = move |_| {
        spawn(async move {
            match conn.stop_injection().await {
                Ok(_) => {
                    inj_polling.set(false);
                    inj_status.with_mut(|s| s.state = InjectionEngineState::Idle);
                    // user-initiated stop bypasses the navbar polling, so reset the sticky toggles here too
                    filter_inj.set(true);
                    pres_timing.set(true);
                }
                Err(e) => action_error.set(Some(e)),
            }
        });
    };

    let on_pause = move |_| {
        spawn(async move {
            if let Err(e) = conn.pause_injection().await {
                action_error.set(Some(e));
            }
        });
    };

    // render

    rsx! {
        document::Link { rel: "stylesheet", href: INJECTION_CSS }
        document::Link { rel: "stylesheet", href: CONFIG_CSS }

        div { class: "injection-page",
            h2 { class: "page-title", "Injection" }

            // source selection
            div { class: "card injection-source",
                h3 { class: "card-title", "Source" }
                p { class: "card-desc",
                    "Select a PCAPNG archive stored on the connected node. \
                     Load Preview to inspect and filter packets before injecting."
                }
                div { class: "injection-source-row",
                    select {
                        class: "form-select injection-archive-select",
                        value: selected.read().clone(),
                        onchange: move |e| selected.set(e.value()),
                        if archive_list.read().is_empty() {
                            {
                                let label = match *conn.state.read() {
                                    ConnectionState::ConnectedBT(_) | ConnectionState::ConnectedIP(_) =>
                                        "No archives found on connected node",
                                    _ => "No archives - connect to a node",
                                };
                                rsx! {
                                    option { value: "", "{label}" }
                                }
                            }
                        }
                        for file in archive_list.read().iter() {
                            option {
                                key: "{file}",
                                value: "{file}",
                                selected: *selected.read() == *file,
                                "{file}"
                            }
                        }
                    }
                    button { class: "btn btn-secondary", onclick: on_refresh, "Refresh" }
                    if !is_ble {
                        button {
                            class: "btn btn-primary",
                            disabled: *load_busy.read() || selected.read().is_empty(),
                            onclick: on_load,
                            if *load_busy.read() {
                                span {
                                    class: "spinner",
                                    style: "width: 14px; height: 14px; border-width: 2px; margin: 0;",
                                }
                            } else {
                                "Load Preview"
                            }
                        }
                    }
                }
                if let Some(err) = load_error.read().clone() {
                    p { class: "injection-error", "{err}" }
                }

                // ip: show parsed packet count after Load Preview
                if !pkts.read().is_empty() {
                    div { class: "injection-count-badge",
                        strong { "{*total.read()}" }
                        " of "
                        strong { "{pkts.read().len()}" }
                        " packets match filter"
                    }
                }

                // ble: show pre-validated count from the node
                if is_ble {
                    {
                        let sel = selected.read().clone();
                        let cnt = counts.read().get(&sel).copied();
                        if let Some(n) = cnt {
                            rsx! {
                                div { class: "injection-count-badge",
                                    strong { "{n}" }
                                    " valid Car2X packets on node"
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }

            // ble: show filter + schedule without packet preview table
            if is_ble && !selected.read().is_empty() {
                div { class: "injection-left",
                    FilterPanel { filter }
                    SchedulePanel {
                        schedule,
                        dry_run,
                        tx_power,
                        pres_timing,
                    }
                    DebugPanel { filter_inj }
                }
            }

            // ip: filter + schedule + packet preview (only after load preview)
            if !is_ble && !pkts.read().is_empty() {
                div { class: "injection-body",
                    div { class: "injection-left",
                        FilterPanel { filter }
                        SchedulePanel {
                            schedule,
                            dry_run,
                            tx_power,
                            pres_timing,
                        }
                        DebugPanel { filter_inj }
                    }
                    PacketTable {
                        summaries: filtered.read().clone(),
                        total_count: *total.read(),
                    }
                }
            }

            // action error
            if let Some(err) = action_error.read().clone() {
                p { class: "injection-error", "{err}" }
            }

            // status + controls (always visible)
            StatusBar {
                status: inj_status.read().clone(),
                can_start: !selected.read().is_empty(),
                on_start,
                on_stop,
                on_pause,
            }
        }
    }
}

// helpers

/// Returns `true` when `pkt` satisfies all active fields in `filter`.
fn matches_filter(pkt: &ParsedPacket, filter: &InjectionFilter, base_ts: i64) -> bool {
    if let Some(vid) = filter.vehicle_id {
        if core_logic::mac_to_station_id(&pkt.mac) != vid { return false; }
    }

    let offset_ms = pkt.timestamp_ms.saturating_sub(base_ts) as u64;
    if let Some(start) = filter.time_range_start_ms {
        if offset_ms < start { return false; }
    }
    if let Some(end) = filter.time_range_end_ms {
        if offset_ms > end { return false; }
    }

    if !filter.protocols.is_empty() {
        match pkt.btp_b_info.as_ref().and_then(|b| port_to_protocol(b.destination_port)) {
            Some(p) if filter.protocols.contains(&p) => {}
            _ => return false,
        }
    }

    true
}

fn port_to_protocol(port: u16) -> Option<ReplayProtocol> {
    match port {
        2001 => Some(ReplayProtocol::Cam),
        2002 => Some(ReplayProtocol::Denm),
        2003 => Some(ReplayProtocol::Mapem),
        2004 => Some(ReplayProtocol::Spatem),
        2006 => Some(ReplayProtocol::Ivim),
        2007 => Some(ReplayProtocol::Srem),
        2008 => Some(ReplayProtocol::Ssem),
        2009 => Some(ReplayProtocol::Cpm),
        _    => None,
    }
}
