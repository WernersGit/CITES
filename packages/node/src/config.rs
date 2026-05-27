use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone)]
pub enum AppMode {
    Local,
    Cloud,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MongoConfig {
    pub uri: String,
    pub database: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InterfacesConfig {
    /// Monitor-mode interface used for passive capture (must carry Radiotap headers).
    pub capture_interface: String,
    /// Interface used for packet injection.  Defaults to `capture_interface` when absent.
    /// For 11p-on-linux: set to the OCB-mode interface (e.g. `wlan0`) so that frames
    /// are actually transmitted over RF while `capture_interface = "mon0"` continues to
    /// record with full Radiotap metadata.
    pub injection_interface: Option<String>,
    pub enable_bluetooth: bool,
    pub enable_network_api: bool,
    pub network_port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub name: String,
    pub mode: AppMode,
    pub interfaces: InterfacesConfig,
    pub mongodb: Option<MongoConfig>,
}

impl AppConfig {
    pub fn load() -> Result<Self, config::ConfigError> {
        let settings = config::Config::builder()
            .add_source(config::File::with_name("config.toml").required(false))
            .add_source(config::Environment::with_prefix("CITES").separator("_"))
            .build()?;

        settings.try_deserialize()
    }
}

// RuntimeConfig

/// Settings pushed at runtime from the client UI, persisted across restarts.
///
/// Stored as JSON at [`RUNTIME_CONFIG_PATH`].  Only fields present in the file
/// override `AppConfig`; absent fields fall back to the static configuration.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RuntimeConfig {
    /// `tracing_subscriber` filter string, e.g. `"debug"` or `"node=info"`.
    pub log_level: Option<String>,
    /// Overrides `interfaces.network_port` from `config.toml`.
    pub api_port: Option<u16>,
}

#[cfg(target_os = "linux")]
const RUNTIME_CONFIG_PATH: &str = "/var/cites-node/runtime_config.json";
#[cfg(not(target_os = "linux"))]
const RUNTIME_CONFIG_PATH: &str = "./runtime_config.json";

impl RuntimeConfig {
    /// Reads the runtime config from disk; returns a default (all `None`) on error.
    pub fn load() -> Self {
        std::fs::read_to_string(RUNTIME_CONFIG_PATH)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Writes the runtime config to disk.
    pub fn save(&self) -> std::io::Result<()> {
        // ensure the directory exists (mainly for Linux target)
        if let Some(parent) = std::path::Path::new(RUNTIME_CONFIG_PATH).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(RUNTIME_CONFIG_PATH, json)
    }
}
