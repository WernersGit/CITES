use dioxus::prelude::*;
use core_logic::pcap_parser::ParsedPacket;

/// A lightweight summary used for preview rendering.
///
/// Built from `ParsedPacket` in the parent memo so the table receives
/// only the fields it displays, avoiding heavy cloning.
#[derive(Clone, PartialEq)]
pub struct PacketSummary {
    pub offset_ms: u64,
    pub mac:       String,
    pub protocol:  &'static str,
    pub data_len:  usize,
}

impl PacketSummary {
    pub fn from_packet(pkt: &ParsedPacket, base_ts: i64) -> Self {
        Self {
            offset_ms: pkt.timestamp_ms.saturating_sub(base_ts) as u64,
            mac:       pkt.mac.clone(),
            protocol:  btp_port_label(pkt.btp_b_info.as_ref().map(|b| b.destination_port)),
            data_len:  pkt.data_len,
        }
    }
}

fn btp_port_label(port: Option<u16>) -> &'static str {
    match port {
        Some(2001) => "CAM",
        Some(2002) => "DENM",
        Some(2003) => "MAPEM",
        Some(2004) => "SPATEM",
        Some(2006) => "IVIM",
        Some(2007) => "SREM",
        Some(2008) => "SSEM",
        Some(2009) => "CPM",
        _          => "-",
    }
}

/// Scrollable packet preview table.
///
/// `summaries` is limited to 200 entries by the caller's memo.
/// `total_count` is the full untruncated count for the count badge.
#[component]
pub fn PacketTable(summaries: Vec<PacketSummary>, total_count: usize) -> Element {
    rsx! {
        div { class: "injection-table-card card",
            div { class: "injection-table-header",
                h3 { class: "card-title", "Packet Preview" }
                span { class: "injection-match-badge",
                    "{total_count} matched"
                    if total_count > summaries.len() {
                        " (showing {summaries.len()})"
                    }
                }
            }

            if summaries.is_empty() {
                p { class: "injection-empty", "No packets match the current filter." }
            } else {
                div { class: "injection-table-wrap",
                    table { class: "injection-table",
                        thead {
                            tr {
                                th { "Offset (ms)" }
                                th { "MAC" }
                                th { "Protocol" }
                                th { "Bytes" }
                            }
                        }
                        tbody {
                            for (i, pkt) in summaries.iter().enumerate() {
                                tr { key: "{i}",
                                    td { class: "mono tabular", "{pkt.offset_ms}" }
                                    td { class: "mono", "{pkt.mac}" }
                                    td {
                                        span {
                                            class: "proto-tag proto-{pkt.protocol.to_lowercase()}",
                                            "{pkt.protocol}"
                                        }
                                    }
                                    td { class: "tabular", "{pkt.data_len}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
