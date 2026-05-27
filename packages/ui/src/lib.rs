// shared UI components for the CITES workspace

use dioxus::prelude::*;
use futures::StreamExt;

mod navbar;
pub use navbar::{Navbar, SidebarLink};

mod sysinfo;
pub use sysinfo::SysInfo;

mod analysis;
pub use analysis::Analysis;

mod home;
pub use home::Home;

pub mod views;
pub use views::config::ConfigView;

pub mod countries;
pub mod source_picker;
pub mod trajectory;

mod live;
pub use live::{LiveView, CarView, MapView, VehicleState};

mod injection;
pub use injection::InjectionView;

mod file_transfer;
pub use file_transfer::FileTransferView;

// persistent settings

const LIVE_TILE_KEY:        &str = "cites.live_tile_url";
const COUNTRY_KEY:          &str = "cites.country_code";
pub(crate) const REPLAY_CONFIG_KEY:   &str = "cites.replay_config";
const TRACKING_WARNING_KEY: &str = "cites.tracking_warning";
const TTS_ENABLED_KEY:      &str = "cites.tts_enabled";
const TTS_LANGUAGE_KEY:     &str = "cites.tts_language";
const NODE_CONFIG_KEY:      &str = "cites.node_config";
const TS_UNIX_FORMAT_KEY:   &str = "cites.ts_unix_format";
const TS_USE_GNW_KEY:       &str = "cites.ts_use_gnw";

/// Sets up the persistent pcap-loop coroutine in the current (layout) scope.
///
/// Must be called from the outermost layout component (`DesktopNavbar` / `WebNavbar`),
/// which is an ancestor of every page, so that the coroutine's CopyValue is always
/// accessible from any child scope without triggering a scope-hoisting warning.
pub fn setup_loop_coroutine() {
    let mut connection = use_context::<platform::ConnectionService>();
    let handle = use_coroutine(move |mut rx: UnboundedReceiver<platform::StartLoopCmd>| async move {
        while let Some(cmd) = rx.next().await {
            let mut svc = connection;
            match cmd {
                platform::StartLoopCmd::Ip(ip) => svc.start_ip_pcap_loop(&ip).await,
                platform::StartLoopCmd::Ble    => svc.start_ble_pcap_loop().await,
            }
        }
    });
    connection.loop_coroutine.set(Some(handle));
}

/// Loads all user settings persisted in `localStorage` into [`platform::ConnectionService`].
///
/// Must be called once from the application layout component, after
/// [`use_context_provider`] has made `ConnectionService` available in scope.
pub fn load_persisted_settings() {
    let conn = use_context::<platform::ConnectionService>();
    let mut tile_url            = conn.live_tile_server_url;
    let mut country_code        = conn.country_code;
    let mut warn_cfg = conn.tracking_warning_cfg;
    let mut tts_enabled         = conn.tts_enabled;
    let mut tts_lang            = conn.tts_language;
    let mut node_cfg            = conn.node_cfg;
    let mut ts_unix_format      = conn.ts_unix_format;
    let mut ts_use_gnw          = conn.ts_use_gnw;

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{LIVE_TILE_KEY}') || '')")
            );
            if let Ok(val) = eval.recv::<String>().await {
                if !val.is_empty() { tile_url.set(val); }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{COUNTRY_KEY}') || '')")
            );
            if let Ok(val) = eval.recv::<String>().await {
                if !val.is_empty() { country_code.set(val); }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{TRACKING_WARNING_KEY}') || '')")
            );
            if let Ok(json) = eval.recv::<String>().await {
                if let Ok(cfg) = serde_json::from_str(&json) {
                    warn_cfg.set(cfg);
                }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{TTS_ENABLED_KEY}') || '')")
            );
            if let Ok(val) = eval.recv::<String>().await {
                if val == "true" { tts_enabled.set(true); }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{TTS_LANGUAGE_KEY}') || '')")
            );
            if let Ok(val) = eval.recv::<String>().await {
                if !val.is_empty() { tts_lang.set(val); }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{NODE_CONFIG_KEY}') || '')")
            );
            if let Ok(json) = eval.recv::<String>().await {
                if let Ok(cfg) = serde_json::from_str(&json) {
                    node_cfg.set(cfg);
                }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{TS_UNIX_FORMAT_KEY}') || '')")
            );
            if let Ok(val) = eval.recv::<String>().await {
                if val == "true" { ts_unix_format.set(true); }
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                &format!("dioxus.send(localStorage.getItem('{TS_USE_GNW_KEY}') || '')")
            );
            if let Ok(val) = eval.recv::<String>().await {
                if val == "true" { ts_use_gnw.set(true); }
            }
        });
    });
}

// localStorage helpers

/// Escapes a string for use as a single-quoted JavaScript string literal.
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Returns a `localStorage.setItem` JS statement for a boolean flag.
fn save_bool_setting(key: &str, value: bool) -> String {
    format!("localStorage.setItem('{}', '{}');", key, if value { "true" } else { "false" })
}

/// Returns a `localStorage.setItem` JS statement for a pre-serialized JSON string.
fn save_json_setting(key: &str, json: &str) -> String {
    format!("localStorage.setItem('{key}', '{}');", escape_js_string(json))
}

/// Returns a `localStorage.setItem` JS statement for a plain string value.
fn save_string_setting(key: &str, value: &str) -> String {
    format!("localStorage.setItem('{key}', '{}');", escape_js_string(value))
}

// public save functions

/// Persists the live-map tile server URL to `localStorage`.
pub fn save_live_tile_url(url: &str) -> String {
    save_string_setting(LIVE_TILE_KEY, url)
}

/// Persists [`core_logic::config::RepeatModeConfig`] as JSON to `localStorage`.
pub fn save_replay_config(cfg: &core_logic::config::RepeatModeConfig) -> String {
    save_json_setting(REPLAY_CONFIG_KEY, &serde_json::to_string(cfg).unwrap_or_default())
}

/// Persists [`core_logic::config::TrackingWarningConfig`] as JSON to `localStorage`.
pub fn save_tracking_warning_config(cfg: &core_logic::config::TrackingWarningConfig) -> String {
    save_json_setting(TRACKING_WARNING_KEY, &serde_json::to_string(cfg).unwrap_or_default())
}

/// Persists [`core_logic::config::NodeConfig`] as JSON to `localStorage`.
pub fn save_node_config(cfg: &core_logic::config::NodeConfig) -> String {
    save_json_setting(NODE_CONFIG_KEY, &serde_json::to_string(cfg).unwrap_or_default())
}

/// Persists the selected country code to `localStorage`.
pub fn save_country_code(code: &str) -> String {
    save_string_setting(COUNTRY_KEY, code)
}

/// Persists the TTS enabled flag to `localStorage`.
pub fn save_tts_enabled(enabled: bool) -> String {
    save_bool_setting(TTS_ENABLED_KEY, enabled)
}

/// Persists the TTS language tag to `localStorage`.
pub fn save_tts_language(lang: &str) -> String {
    save_string_setting(TTS_LANGUAGE_KEY, lang)
}

/// Persists the timestamp display format flag to `localStorage`.
pub fn save_ts_unix_format(enabled: bool) -> String {
    save_bool_setting(TS_UNIX_FORMAT_KEY, enabled)
}

/// Persists the timestamp source flag to `localStorage`.
pub fn save_ts_use_gnw(enabled: bool) -> String {
    save_bool_setting(TS_USE_GNW_KEY, enabled)
}

/// Builds the MapLibre style URL from a tileserver base URL.
///
/// Appends `styles/basic-preview/style.json` when the base URL has no path
/// component beyond host:port; otherwise the URL is used as-is.
pub fn style_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let has_path = base
        .find("://")
        .map(|i| base[i + 3..].contains('/'))
        .unwrap_or(false);
    if has_path {
        base.to_string()
    } else {
        format!("{}/styles/basic-preview/style.json", base)
    }
}
