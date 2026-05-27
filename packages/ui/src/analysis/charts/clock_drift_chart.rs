use dioxus::prelude::*;
use super::ClockDriftSample;

#[derive(Props, Clone, PartialEq)]
pub struct ClockDriftChartProps {
    pub samples: Vec<ClockDriftSample>,
    pub mac_order: Vec<String>,
    pub min_ms: i64,
    pub max_ms: i64,
}

/// Clock Drift Tracking — matches Python CamAnalyzer.py report 3.
///
/// The Python computes drift = (mactime - ieee1609dot2.generationTime) - global_offset,
/// smoothed with rolling median window=20, plotted in ms.
/// This requires `ieee1609dot2.generationTime` (absolute TAI timestamp from the ITS
/// security layer in µs) which the current parser does not yet decode.
#[component]
pub fn ClockDriftChart(props: ClockDriftChartProps) -> Element {
    rsx! {
        div {
            style: "display: flex; flex-direction: column; align-items: center; justify-content: center; \
                    min-height: 140px; background: #f8f9fa; border-radius: 6px; gap: 8px; padding: 1.2rem; text-align: center;",
            span { style: "font-size: 1.5rem;", "⏱" }
            span { style: "font-size: 0.88rem; font-weight: 600; color: #495057;",
                "Clock Drift Analysis not available"
            }
            span { style: "font-size: 0.78rem; color: #888; max-width: 360px; line-height: 1.5;",
                "Requires "
                code { style: "background: #e9ecef; padding: 1px 4px; border-radius: 3px;",
                    "ieee1609dot2.generationTime"
                }
                " — the absolute TAI timestamp embedded in the ITS security wrapper. \
                 Add ieee1609dot2 security-header decoding to the parser to enable this chart."
            }
        }
    }
}
