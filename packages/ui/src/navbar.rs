use dioxus::prelude::*;
use platform::ConnectionState;
use core_logic::config::InjectionEngineState;

const NAVBAR_CSS: Asset = asset!("/assets/styling/navbar.css");

#[component]
pub fn Navbar(children: Element) -> Element {
    let mut is_open = use_signal(|| false);
    provide_context(is_open);

    let mut connection = use_context::<platform::ConnectionService>();

    // Global TTS drain: always clears the queue, speaks only when TTS is enabeld.
    use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            async_std::task::sleep(std::time::Duration::from_millis(200)).await;
            let pending: Vec<String> = {
                let mut q = connection.tts_queue.write();
                std::mem::take(&mut *q)
            };
            if *connection.tts_enabled.read() {
                let lang = connection.tts_language.read().clone();
                let lang_ref = if lang.is_empty() { None } else { Some(lang.as_str()) };
                for text in &pending {
                    let _ = platform::tts::speak(text, lang_ref);
                }
            }
        }
    });

    // Injection status polling: active only while inj_polling == true (1500 ms interval).
    // On run end (transition to a non-active state) the filter toggle reverts to its default so the next run starts predictably.
    use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            async_std::task::sleep(std::time::Duration::from_millis(1_500)).await;
            if !*connection.inj_polling.read() { continue; }
            if let Ok(s) = connection.fetch_injection_status().await {
                let done = !matches!(
                    s.state,
                    InjectionEngineState::Running | InjectionEngineState::Paused
                );
                // mirror the server-reported filter state into the UI signal so the toggle is always in lock-step with the node
                if !done && *connection.filter_inj.read() != s.filter_inj {
                    connection.filter_inj.set(s.filter_inj);
                }
                connection.inj_status.set(s);
                if done {
                    connection.inj_polling.set(false);
                    connection.filter_inj.set(true);
                    connection.pres_timing.set(true);
                }
            }
        }
    });

    // Log-level conflict detection: polls connection state every 500 ms.
    // When the state transitions to Connected, fetches the node's NodeConfig and compares its log level with the client's configured level.
    use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        let mut was_connected = false;
        loop {
            async_std::task::sleep(std::time::Duration::from_millis(500)).await;

            let now_connected = matches!(
                *connection.state.read(),
                ConnectionState::ConnectedIP(_) | ConnectionState::ConnectedBT(_)
            );

            if now_connected && !was_connected {
                if let Ok(node_config) = connection.fetch_node_config().await {
                    let client_level = connection.node_cfg.read().log_level;
                    if node_config.log_level != client_level {
                        connection.log_level_conflict.set(Some(node_config.log_level));
                    }
                }
            }

            was_connected = now_connected;
        }
    });

    // read conflict state for the modal
    let conflict = *connection.log_level_conflict.read();

    rsx! {
        document::Link { rel: "stylesheet", href: NAVBAR_CSS }

        button {
            class: "hamburger-btn",
            onclick: move |_| is_open.with_mut(|v| *v = !*v),
            span { class: "hamburger-line" }
            span { class: "hamburger-line" }
            span { class: "hamburger-line" }
        }

        div { class: if is_open() { "sidebar open" } else { "sidebar" }, {children} }

        // log-level conflict modal - shown regardless of which page is active
        if let Some(node_level) = conflict {
            div { class: "modal-backdrop",
                div { class: "modal-box",
                    h3 { class: "modal-title", "Log Level Conflict" }
                    p { class: "modal-body",
                        "The connected node uses log level "
                        strong { "{node_level.label()}" }
                        ", but the client is configured for "
                        strong { "{connection.node_cfg.read().log_level.label()}" }
                        ". Which setting should be applied?"
                    }
                    div { class: "modal-actions",
                        //adopt node's level -> update client config
                        button {
                            class: "btn btn-secondary",
                            onclick: move |_| {
                                connection.node_cfg.write().log_level = node_level;
                                let cfg = connection.node_cfg.read().clone();
                                let js = crate::save_node_config(&cfg);
                                spawn(async move {
                                    let _ = document::eval(&js).await;
                                });
                                connection.log_level_conflict.set(None);
                            },
                            "Use Node Setting ({node_level.label()})"
                        }
                        // keep client's level -> push to node
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                let cfg = connection.node_cfg.read().clone();
                                connection.log_level_conflict.set(None);
                                spawn(async move {
                                    let _ = connection.push_node_config(cfg).await;
                                });
                            },
                            "Use Client Setting ({connection.node_cfg.read().log_level.label()})"
                        }
                    }
                }
            }
        }
    }
}

/// Wrapper around `Link` that auto-closes the sidebar on click
#[component]
pub fn SidebarLink<R>(to: R, children: Element) -> Element
where
    R: Routable + Clone + std::fmt::Display + PartialEq + 'static,
    <R as std::str::FromStr>::Err: std::fmt::Display,
{
    let mut is_open = try_consume_context::<Signal<bool>>();

    rsx! {
        Link {
            to,
            onclick: move |_| {
                if let Some(mut is_open) = is_open {
                    is_open.set(false);
                }
            },
            {children}
        }
    }
}
