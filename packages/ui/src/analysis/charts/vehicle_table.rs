use dioxus::prelude::*;
use super::{VehicleRow, COLORS, fmt_hms};

#[derive(Props, Clone, PartialEq)]
pub struct VehicleTableProps {
    pub vehicles: Vec<VehicleRow>,
}

#[component]
pub fn VehicleTable(props: VehicleTableProps) -> Element {
    if props.vehicles.is_empty() {
        return rsx! { p { style: "color: #999; font-size: 0.85rem;", "No virtual vehicles detected." } };
    }

    let mut rows = props.vehicles.clone();
    rows.sort_by_key(|r| r.virtual_id);

    rsx! {
        div { style: "overflow-x: auto;",
            table {
                style: "width: 100%; border-collapse: collapse; font-size: 0.82rem; font-family: sans-serif; color: #212529;",
                thead {
                    tr {
                        style: "background: #f1f3f5; text-align: left;",
                        for col in ["Virtual ID", "MAC Sequence", "Start", "End", "Packets"] {
                            th { style: "padding: 6px 10px; border-bottom: 2px solid #dee2e6; white-space: nowrap; font-weight: 600;", "{col}" }
                        }
                    }
                }
                tbody {
                    for (i, row) in rows.iter().enumerate() {
                        tr {
                            key: "{row.virtual_id}",
                            style: if i % 2 == 0 { "background: #fff;" } else { "background: #f8f9fa;" },
                            td { style: "padding: 5px 10px; border-bottom: 1px solid #eee; text-align: center; font-weight: 600; color: {COLORS[row.virtual_id as usize % COLORS.len()]};",
                                "{row.virtual_id}"
                            }
                            td { style: "padding: 5px 10px; border-bottom: 1px solid #eee; font-family: monospace; font-size: 0.78rem; max-width: 380px; word-break: break-all; color: #212529;",
                                "{row.macs.join(\" → \")}"
                            }
                            td { style: "padding: 5px 10px; border-bottom: 1px solid #eee; white-space: nowrap; font-family: monospace; color: #212529;",
                                "{fmt_hms(row.start_ms)}"
                            }
                            td { style: "padding: 5px 10px; border-bottom: 1px solid #eee; white-space: nowrap; font-family: monospace; color: #212529;",
                                "{fmt_hms(row.end_ms)}"
                            }
                            td { style: "padding: 5px 10px; border-bottom: 1px solid #eee; text-align: right; color: #212529;",
                                "{row.packet_count}"
                            }
                        }
                    }
                }
            }
        }
    }
}
