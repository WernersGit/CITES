use dioxus::prelude::*;
use platform::ConnectionService;

#[derive(PartialEq, Clone, Props)]
struct ChartProps {
    title: String,
    data: Vec<f64>,
    unit: String,
    max_label: Option<f64>,
}

#[component]
fn MetricChart(props: ChartProps) -> Element {
    let current = props.data.last().copied().unwrap_or(0.0);

    let min_val = 0.0;
    
    // evaluate the actual max data point
    let data_max = props.data.iter().copied().fold(0.0_f64, f64::max);
    
    let max_val = props.max_label.unwrap_or(10.0);
    let mid_val = max_val / 2.0;
    let min_val = 0.0;

    let mid_val = (min_val + max_val) / 2.0;

    let width = 600.0;
    let height = 200.0;

    let range = (max_val - min_val).max(1.0);

    let points: String = props
        .data
        .iter()
        .enumerate()
        .map(|(i, &val)| {
            let x = (i as f64 / (props.data.len() - 1).max(1) as f64) * width;
            let y = height - ((val - min_val) / range) * height;
            format!("{x},{y}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    rsx! {
        div {
            class: "chart-container",
            style: "margin-bottom: 4rem; width: 100%; max-width: 650px;",

            div { style: "display: flex; align-items: stretch; gap: 0.5rem;",
                // Y-Axis
                div { style: "display: flex; flex-direction: column; justify-content: space-between; height: 200px; font-size: 0.8rem; text-align: right; color: #666; width: 45px;",
                    span { "{max_val:.1}" }
                    span { "{mid_val:.1}" }
                    span { "{min_val:.1}" }
                }

                // Graph
                div { style: "border-left: 2px solid #333; border-bottom: 2px solid #333; width: 100%; height: 200px; position: relative;",
                    svg {
                        width: "100%",
                        height: "100%",
                        view_box: "0 0 {width} {height}",
                        preserve_aspect_ratio: "none",
                        polyline {
                            points: "{points}",
                            fill: "none",
                            stroke: "#007bff",
                            stroke_width: "2",
                        }
                    }
                }
            }

            // Current value
            div { style: "font-weight: bold; font-size: 1.2rem; margin-top: 1rem; text-align: center; margin-left: 45px;",
                "{props.title}: "
                span { style: "color: #007bff;", "{current as u32} {props.unit}" }
            }
        }
    }
}

#[component]
pub fn SysInfo() -> Element {
    let connection = use_context::<ConnectionService>();

    // Hardware history vectors: 240 slots for 2 minutes of data at 500ms intervals
    let mut cpu_history = use_signal(|| vec![0.0; 240]);
    let mut temp_history = use_signal(|| vec![0.0; 240]);
    let mut ram_history = use_signal(|| vec![0u32; 240]);
    let mut total_ram = use_signal(|| 8192u32); // default to 8GB

    // Polls the unified /status endpoint every 2 s.
    // Using fetch_status (vs. the old fetch_metrics) avoids a separate
    // /replay/count request when the Config view is also active, because
    // both views hit the same cached endpoint on the node.
    use_effect(move || {
        spawn(async move {
            loop {
                match connection.fetch_status().await {
                    Ok(status) => {
                        *total_ram.write() = status.ram_total_mb.round() as u32;

                        cpu_history.write().remove(0);
                        cpu_history.write().push(status.cpu_usage.round() as f64);

                        ram_history.write().remove(0);
                        ram_history.write().push(status.ram_used_mb.round() as u32);

                        temp_history.write().remove(0);
                        temp_history.write().push(status.temp_celsius.round() as f64);
                    }
                    Err(e) => {
                        println!("SysInfo fetch error: {}", e);
                    }
                }
                async_std::task::sleep(std::time::Duration::from_millis(2000)).await;
            }
        });
    });

    rsx! {
        div {
            class: "sysinfo-container",
            style: "padding: var(--top-bar) 2rem 2rem var(--page-left); max-width: 900px; font-family: sans-serif;",
            h1 { class: "page-title", "System Information" }

            div { class: "hardware-history-section",

                MetricChart {
                    title: "CPU Usage",
                    data: cpu_history(),
                    unit: "%",
                    max_label: Some(100.0),
                }

                MetricChart {
                    title: "RAM Usage",
                    data: ram_history().iter().map(|&v| v as f64).collect(),
                    unit: "MB",
                    max_label: Some(8192.0),
                }

                MetricChart {
                    title: "Temperature",
                    data: temp_history(),
                    unit: "°C",
                    max_label: Some(70.0),
                }
            }
        }
    }
}
