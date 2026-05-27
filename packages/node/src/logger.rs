use std::sync::OnceLock;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, reload};

#[cfg(target_os = "linux")]
const LOG_DIR: &str = "/var/log/cites-node";
#[cfg(not(target_os = "linux"))]
const LOG_DIR: &str = "./logs";

const LOG_FILE: &str = "cites-node.log";

// type-erased setter so we never nedd to name the complex genreic Handle type
type SetLevelFn = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
static SET_LEVEL: OnceLock<SetLevelFn> = OnceLock::new();

/// Keeps the background file-writer thread alive.
/// Drop at the end of `main` to flush and close the log file.
pub struct LogGuard {
    _file_guard: WorkerGuard,
}

pub struct NodeLogger;

impl NodeLogger {
    /// Initialises the global `tracing` subscriber with two sinks:
    /// - **stderr** — coloured output (systemd journal / interactive)
    /// - **file**   — daily-rotating plain-text log in [`LOG_DIR`]
    ///
    /// Log level defaults to `INFO`; override with `RUST_LOG` or call [`NodeLogger::set_level`] at runtime.
    pub fn init() -> LogGuard {
        if let Err(e) = std::fs::create_dir_all(LOG_DIR) {
            eprintln!("[NodeLogger] cannot create log dir {}: {}", LOG_DIR, e);
        }

        let file_appender = tracing_appender::rolling::daily(LOG_DIR, LOG_FILE);
        let (non_blocking_file, file_guard) = tracing_appender::non_blocking(file_appender);

        let initial_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"));

        let (reload_layer, handle) = reload::Layer::new(initial_filter);

        // store a type-erased setter
        SET_LEVEL.set(Box::new(move |level_str: &str| {
            handle
                .reload(EnvFilter::new(level_str))
                .map_err(|e| e.to_string())
        })).ok();

        tracing_subscriber::registry()
            .with(reload_layer)
            .with(fmt::layer().with_ansi(true))
            .with(fmt::layer().with_ansi(false).with_writer(non_blocking_file))
            .init();

        LogGuard { _file_guard: file_guard }
    }

    /// Changes the active log level without restarting the process.
    ///
    /// `level_str` is a `tracing_subscriber::EnvFilter` string, e.g. `"debug"`, `"info"`, `"warn"`, `"error"`, or a per-module filter like `"node=debug"`
    pub fn set_level(level_str: &str) -> Result<(), String> {
        SET_LEVEL
            .get()
            .ok_or_else(|| "Logger not initialized".to_string())
            .and_then(|f| f(level_str))
    }
}
