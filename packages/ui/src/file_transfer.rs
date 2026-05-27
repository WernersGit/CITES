use dioxus::prelude::*;
use platform::{ConnectionService, ConnectionState};

const FILE_TRANSFER_CSS: Asset = asset!("/assets/styling/file_transfer.css");

#[component]
pub fn FileTransferView() -> Element {
    let conn = use_context::<ConnectionService>();

    // downlod state
    let mut archives     = use_signal(Vec::<String>::new);
    let mut selected = use_signal(String::new);
    let mut download_busy    = use_signal(|| false);
    let mut download_error   = use_signal(|| Option::<String>::None);
    let mut download_ok      = use_signal(|| false);

    // upload state
    let mut pending: Signal<Option<(String, Vec<u8>)>> = use_signal(|| None);
    let mut upload_busy  = use_signal(|| false);
    let mut upload_error = use_signal(|| Option::<String>::None);
    let mut upload_ok    = use_signal(|| false);

    // always declared; only mutated by the desktop/web drag-and-dropo effect
    let mut drag_active = use_signal(|| false);

    use_effect(move || {
        spawn(async move {
            if let Ok(list) = conn.fetch_archive_list().await {
                if let Some(first) = list.first() {
                    selected.set(first.clone());
                }
                archives.set(list);
            }
        });
    });

    //drag-and-drop JS setup - desktop/web only
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(r#"
                function setupDropzone() {
                    const zone = document.getElementById('ft-dropzone');
                    if (!zone) { setTimeout(setupDropzone, 100); return; }
                    zone.addEventListener('dragenter', (e) => {
                        e.preventDefault();
                        dioxus.send({type: 'dragenter'});
                    });
                    zone.addEventListener('dragleave', (e) => {
                        e.preventDefault();
                        dioxus.send({type: 'dragleave'});
                    });
                    zone.addEventListener('dragover', (e) => { e.preventDefault(); });
                    zone.addEventListener('drop', (e) => {
                        e.preventDefault();
                        const file = e.dataTransfer.files[0];
                        if (!file) { dioxus.send({type: 'dragleave'}); return; }
                        const reader = new FileReader();
                        reader.onload = (evt) => {
                            const bytes = new Uint8Array(evt.target.result);
                            const chunk = 65536;
                            let binary = '';
                            for (let i = 0; i < bytes.length; i += chunk) {
                                binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
                            }
                            dioxus.send({type: 'drop', name: file.name, data: btoa(binary)});
                        };
                        reader.readAsArrayBuffer(file);
                    });
                    dioxus.send({type: 'ready'});
                }
                setupDropzone();
            "#);

            loop {
                match eval.recv::<serde_json::Value>().await {
                    Ok(val) => match val["type"].as_str() {
                        Some("dragenter") => drag_active.set(true),
                        Some("dragleave") => drag_active.set(false),
                        Some("drop") => {
                            drag_active.set(false);
                            if let (Some(name), Some(b64)) =
                                (val["name"].as_str(), val["data"].as_str())
                            {
                                use base64::Engine;
                                if let Ok(bytes) =
                                    base64::engine::general_purpose::STANDARD.decode(b64)
                                {
                                    pending.set(Some((name.to_string(), bytes)));
                                    upload_error.set(None);
                                    upload_ok.set(false);
                                }
                            }
                        }
                        _ => {}
                    },
                    Err(_) => break,
                }
            }
        });
    });

    // handlers

    let on_refresh = move |_| {
        spawn(async move {
            if let Ok(list) = conn.fetch_archive_list().await {
                archives.set(list);
            }
        });
    };

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let on_download = move |_: Event<MouseData>| {
        let fname = selected.read().clone();
        if fname.is_empty() { return; }
        spawn(async move {
            download_busy.set(true);
            download_error.set(None);
            download_ok.set(false);
            match conn.fetch_archive_file(&fname).await {
                Ok(bytes) => {
                    let save = rfd::AsyncFileDialog::new()
                        .set_file_name(&fname)
                        .add_filter("PCAPNG", &["pcapng", "pcap"])
                        .save_file()
                        .await;
                    if let Some(file) = save {
                        match file.write(&bytes).await {
                            Ok(_)  => download_ok.set(true),
                            Err(e) => download_error.set(Some(e.to_string())),
                        }
                    }
                }
                Err(e) => download_error.set(Some(e)),
            }
            download_busy.set(false);
        });
    };

    // mobile: write to the OS temp directory; path is shown in the success message
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let on_download = move |_: Event<MouseData>| {
        let fname = selected.read().clone();
        if fname.is_empty() { return; }
        spawn(async move {
            download_busy.set(true);
            download_error.set(None);
            download_ok.set(false);
            match conn.fetch_archive_file(&fname).await {
                Ok(bytes) => {
                    let path = std::env::temp_dir().join(&fname);
                    match std::fs::write(&path, &bytes) {
                        Ok(_)  => download_ok.set(true),
                        Err(e) => download_error.set(Some(e.to_string())),
                    }
                }
                Err(e) => download_error.set(Some(e)),
            }
            download_busy.set(false);
        });
    };

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let on_pick_file = move |_: Event<MouseData>| {
        spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("PCAPNG", &["pcapng", "pcap"])
                .pick_file()
                .await;
            if let Some(file) = picked {
                let name = file.file_name();
                let data = file.read().await;
                pending.set(Some((name, data)));
                upload_error.set(None);
                upload_ok.set(false);
            }
        });
    };

    let on_upload = move |_| {
        let Some((name, data)) = pending.read().clone() else { return; };
        spawn(async move {
            upload_busy.set(true);
            upload_error.set(None);
            upload_ok.set(false);
            match conn.upload_archive_file(&name, data).await {
                Ok(_) => {
                    upload_ok.set(true);
                    pending.set(None);
                    if let Ok(list) = conn.fetch_archive_list().await {
                        archives.set(list);
                    }
                }
                Err(e) => upload_error.set(Some(e)),
            }
            upload_busy.set(false);
        });
    };

    // upload zone differs by platform

    #[cfg(any(target_os = "android", target_os = "ios"))]
    let zone = rsx! {
        label { r#for: "ft-file-input", class: "ft-dropzone",
            DropZoneContent { file: pending.read().clone(), hint: "Tap to select file" }
        }
        input {
            id: "ft-file-input",
            r#type: "file",
            accept: ".pcapng,.pcap",
            style: "display:none",
            onchange: move |evt: FormEvent| {
                if let Some(file) = evt.files().into_iter().next() {
                    spawn(async move {
                        if let Ok(bytes) = file.read_bytes().await {
                            pending.set(Some((file.name(), bytes.to_vec())));
                            upload_error.set(None);
                            upload_ok.set(false);
                        }
                    });
                }
            },
        }
    };

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let zone = rsx! {
        div {
            id: "ft-dropzone",
            class: if *drag_active.read() { "ft-dropzone ft-dropzone-active" } else { "ft-dropzone" },
            onclick: on_pick_file,
            DropZoneContent {
                file: pending.read().clone(),
                hint: "Drop file here or click to select",
            }
        }
    };

    // render

    let is_connected = matches!(
        *conn.state.read(),
        ConnectionState::ConnectedBT(_) | ConnectionState::ConnectedIP(_)
    );

    let save_path = {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        { std::env::temp_dir().join(selected.read().as_str()).display().to_string() }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        { String::new() }
    };

    rsx! {
        document::Link { rel: "stylesheet", href: FILE_TRANSFER_CSS }

        div { class: "ft-page",
            h2 { class: "page-title", "File Transfer" }

            // download
            div { class: "card ft-card",
                h3 { class: "card-title", "Download from Node" }
                p { class: "card-desc",
                    "Select a PCAPNG archive stored on the connected node and save it locally."
                }

                div { class: "ft-row",
                    select {
                        class: "form-select ft-archive-select",
                        value: selected.read().clone(),
                        onchange: move |e| {
                            selected.set(e.value());
                            download_error.set(None);
                            download_ok.set(false);
                        },

                        if archives.read().is_empty() {
                            {
                                let label = if is_connected {
                                    "No archives found on connected node"
                                } else {
                                    "No archives — connect to a node"
                                };
                                rsx! {
                                    option { value: "", "{label}" }
                                }
                            }
                        }
                        for file in archives.read().iter() {
                            option {
                                key: "{file}",
                                value: "{file}",
                                selected: *selected.read() == *file,
                                "{file}"
                            }
                        }
                    }

                    button { class: "btn btn-secondary", onclick: on_refresh, "Refresh" }

                    button {
                        class: "btn btn-primary",
                        disabled: *download_busy.read() || selected.read().is_empty(),
                        onclick: on_download,
                        if *download_busy.read() {
                            span {
                                class: "spinner",
                                style: "width:14px;height:14px;border-width:2px;margin:0;",
                            }
                        } else {
                            "Download"
                        }
                    }
                }

                if let Some(err) = download_error.read().clone() {
                    p { class: "ft-msg ft-error", "{err}" }
                }
                if *download_ok.read() {
                    if save_path.is_empty() {
                        p { class: "ft-msg ft-ok", "File saved successfully." }
                    } else {
                        p { class: "ft-msg ft-ok", "Saved to {save_path}" }
                    }
                }
            }

            // upload
            div { class: "card ft-card",
                h3 { class: "card-title", "Upload to Node" }
                p { class: "card-desc", "Upload a local PCAPNG archive to the connected node." }

                {zone}

                if pending.read().is_some() {
                    div { class: "ft-upload-row",
                        button {
                            class: "btn btn-primary",
                            disabled: *upload_busy.read() || !is_connected,
                            onclick: on_upload,
                            if *upload_busy.read() {
                                span {
                                    class: "spinner",
                                    style: "width:14px;height:14px;border-width:2px;margin:0;",
                                }
                            } else {
                                "Upload to Node"
                            }
                        }
                        button {
                            class: "btn btn-secondary",
                            disabled: *upload_busy.read(),
                            onclick: move |_| {
                                pending.set(None);
                                upload_ok.set(false);
                                upload_error.set(None);
                            },
                            "Clear"
                        }
                    }
                }

                if let Some(err) = upload_error.read().clone() {
                    p { class: "ft-msg ft-error", "{err}" }
                }
                if *upload_ok.read() {
                    p { class: "ft-msg ft-ok", "File uploaded successfully." }
                }
            }
        }
    }
}

/// shared content for both mobile and desktop drop zones
#[component]
fn DropZoneContent(file: Option<(String, Vec<u8>)>, hint: &'static str) -> Element {
    rsx! {
        span { class: "ft-drop-icon", "↑" }
        if let Some((name, data)) = file {
            p { class: "ft-drop-filename", "{name}" }
            {
                let n = data.len();
                let size_str = if n >= 1_048_576 {
                    format!("{:.1} MB", n as f64 / 1_048_576.0)
                } else if n >= 1024 {
                    format!("{:.1} KB", n as f64 / 1024.0)
                } else {
                    format!("{n} B")
                };
                rsx! {
                    p { class: "ft-drop-size", "{size_str}" }
                }
            }
        } else {
            p { class: "ft-drop-hint", "{hint}" }
            p { class: "ft-drop-sub", ".pcapng / .pcap" }
        }
    }
}
