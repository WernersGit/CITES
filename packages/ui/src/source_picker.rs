use dioxus::prelude::*;
use platform::ConnectionService;
use core_logic::pcap_parser::{PcapParser, ParsedPacket};

const LOCAL_CAPTURE_DIR: &str = "./captures";
const ARCHIVE_FILE_PREFIX: &str = "archive_";

#[derive(Clone, PartialEq)]
enum PickerState {
    SelectSource,
    NodeList { files: Vec<String> },
    ClientList { files: Vec<String> },
    Loading(String),
    Error(String),
}

#[derive(Props, Clone, PartialEq)]
pub struct SourcePickerProps {
    pub on_dismiss: EventHandler<()>,
    pub on_loaded: EventHandler<Vec<ParsedPacket>>,
}

#[component]
pub fn SourcePicker(props: SourcePickerProps) -> Element {
    let mut state = use_signal(|| PickerState::SelectSource);
    let connection = use_context::<ConnectionService>();

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let external_file_btn = {
        let on_loaded = props.on_loaded.clone();
        let on_open = move |_| {
            let on_loaded = on_loaded.clone();
            let mut state = state.clone();
            spawn(async move {
                let picked = rfd::AsyncFileDialog::new()
                    .add_filter("PCAPNG", &["pcapng", "pcap"])
                    .pick_file()
                    .await;
                if let Some(file) = picked {
                    *state.write() = PickerState::Loading(format!("Loading {}…", file.file_name()));
                    let data = file.read().await;
                    match PcapParser::parse_bytes(&data) {
                        Ok(packets) => on_loaded.call(packets),
                        Err(e) => *state.write() = PickerState::Error(e.to_string()),
                    }
                }
            });
        };
        rsx! {
            source_button {
                label: "Open external file",
                description: "OS file picker – any PCAPNG file on this device",
                onclick: on_open,
            }
        }
    };
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let external_file_btn = rsx! {};

    // list archivs on the connected node
    let on_browse_node = {
        move |_| {
            let mut st = state.clone();
            spawn(async move {
                *st.write() = PickerState::Loading("Fetching file list from node…".into());
                match connection.fetch_archive_list().await {
                    Ok(files) => *st.write() = PickerState::NodeList { files },
                    Err(e) => *st.write() = PickerState::Error(e),
                }
            });
        }
    };

    //local archives from ./captures
    let on_browse_client = move |_| {
        let dir = std::path::Path::new(LOCAL_CAPTURE_DIR);
        let mut files: Vec<String> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with(ARCHIVE_FILE_PREFIX))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        };
        files.sort();
        *state.write() = PickerState::ClientList { files };
    };

    let content = match state.read().clone() {
        PickerState::SelectSource => rsx! {
            p { style: "margin: 0 0 1.5rem; color: #555; font-size: 0.9rem;",
                "Select a data source for offline analysis:"
            }
            div { style: "display: flex; flex-direction: column; gap: 10px;",
                {external_file_btn}
                source_button {
                    label: "Locally stored recordings",
                    description: "PCAPNG archives saved on this device",
                    onclick: on_browse_client,
                }
                source_button {
                    label: "Recordings on the node",
                    description: "Load archived recordings from the connected node",
                    onclick: on_browse_node,
                }
            }
        },

        PickerState::NodeList { files } | PickerState::ClientList { files } => {
            let is_node = matches!(state.read().clone(), PickerState::NodeList { .. });
            rsx! {
                p { style: "margin: 0 0 0.75rem; font-size: 0.85rem; color: #555;",
                    if is_node {
                        "Recordings stored on the node:"
                    } else {
                        "Locally stored recordings:"
                    }
                }
                if files.is_empty() {
                    p { style: "color: #999; font-style: italic;", "No files found." }
                }
                div { style: "max-height: 300px; overflow-y: auto; border: 1px solid #dee2e6; border-radius: 6px;",
                    for filename in files.iter() {
                        file_row {
                            key: "{filename}",
                            filename: filename.clone(),
                            is_node,
                            on_select: {
                                let filename = filename.clone();
                                let on_loaded = props.on_loaded.clone();
                                move |_| {
                                    let filename = filename.clone();
                                    let on_loaded = on_loaded.clone();
                                    let mut state = state.clone();
                                    spawn(async move {
                                        *state.write() = PickerState::Loading(
                                            format!("Loading {}…", filename),
                                        );
                                        let result = if is_node {
                                            connection.fetch_archive_file(&filename).await
                                        } else {
                                            let path = std::path::Path::new(LOCAL_CAPTURE_DIR).join(&filename);
                                            std::fs::read(&path).map_err(|e| e.to_string())
                                        };
                                        match result {
                                            Ok(data) => {
                                                match PcapParser::parse_bytes(&data) {
                                                    Ok(pkts) => on_loaded.call(pkts),
                                                    Err(e) => *state.write() = PickerState::Error(e.to_string()),
                                                }
                                            }
                                            Err(e) => *state.write() = PickerState::Error(e),
                                        }
                                    });
                                }
                            },
                        }
                    }
                }
                button {
                    style: "margin-top: 12px; padding: 6px 16px; border: 1px solid #aaa; border-radius: 6px; \
                            background: #f8f9fa; cursor: pointer; font-size: 0.85rem;",
                    onclick: move |_| {
                        *state.write() = PickerState::SelectSource;
                    },
                    "Back"
                }
            }
        },

        PickerState::Loading(msg) => rsx! {
            div { style: "display: flex; align-items: center; gap: 12px; padding: 1rem 0;",
                span { "{msg}" }
            }
        },

        PickerState::Error(msg) => rsx! {
            p { style: "color: #dc3545; margin: 0 0 1rem;", "Error: {msg}" }
            button {
                style: "padding: 6px 16px; border: 1px solid #aaa; border-radius: 6px; \
                        background: #f8f9fa; cursor: pointer;",
                onclick: move |_| {
                    *state.write() = PickerState::SelectSource;
                },
                "Back"
            }
        },
    };

    rsx! {
        div {
            style: "position: fixed; inset: 0; background: rgba(0,0,0,0.45); \
                    display: flex; align-items: center; justify-content: center; z-index: 1000;",
            onclick: move |_| props.on_dismiss.call(()),

            div {
                style: "background: #fff; border-radius: 12px; padding: 2rem; \
                        min-width: 480px; max-width: 560px; box-shadow: 0 8px 32px rgba(0,0,0,0.18);",
                onclick: move |e| e.stop_propagation(),

                div { style: "display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.25rem;",
                    h2 { style: "margin: 0; font-size: 1.2rem; color: #111;", "Select data source" }
                    button {
                        style: "background: none; border: none; font-size: 1.4rem; cursor: pointer; color: #888; line-height: 1;",
                        onclick: move |_| props.on_dismiss.call(()),
                        "X"
                    }
                }

                {content}
            }
        }
    }
}

// helpers

#[derive(Props, Clone, PartialEq)]
struct SourceButtonProps {
    label: String,
    description: String,
    onclick: EventHandler<MouseEvent>,
}

#[component]
fn source_button(props: SourceButtonProps) -> Element {
    rsx! {
        button {
            style: "text-align: left; padding: 12px 16px; border: 1px solid #dee2e6; \
                    border-radius: 8px; background: #f8f9fa; cursor: pointer; width: 100%;",
            onclick: move |e| props.onclick.call(e),
            div { style: "font-weight: 600; margin-bottom: 2px;", "{props.label}" }
            div { style: "font-size: 0.82rem; color: #666;", "{props.description}" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FileRowProps {
    filename: String,
    is_node: bool,
    on_select: EventHandler<MouseEvent>,
}

#[component]
fn file_row(props: FileRowProps) -> Element {
    rsx! {
        button {
            style: "display: block; width: 100%; text-align: left; padding: 8px 12px; \
                    border: none; border-bottom: 1px solid #dee2e6; background: #fff; \
                    cursor: pointer; font-family: monospace; font-size: 0.85rem;",
            onclick: move |e| props.on_select.call(e),
            "{props.filename}"
        }
    }
}
