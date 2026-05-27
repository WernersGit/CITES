use std::sync::{OnceLock, mpsc};
use std::time::Duration;
use tts::Tts;

/// All TTS announcemnts the application can make.
///
/// Call [`TtsMessage::text`] with a BCP 47 language prefix (`"de"` or `"en"`)
/// to obtain the localised string.  An empty prefix falls back to English.
#[derive(Debug, Clone)]
pub enum TtsMessage {
    /// Ego MAC changed; inner value is the pre-formatted spoken MAC string.
    EgoMac(String),
    ForeignVehicleFound,
    ForeignVehicleLost,
    TrackingDurationWarning,
    TrackingDistanceWarning,
    Test,
}

impl TtsMessage {
    /// Returns the localised announcement text for the given language prefix.
    pub fn text(&self, lang: &str) -> String {
        let de = lang.starts_with("de");
        let s: &str = match self {
            Self::EgoMac(mac) => return format!("Ego: {mac}"),
            Self::ForeignVehicleFound     => if de { "Fremdfahrzeug gefunden" }               else { "Foreign vehicle detected" },
            Self::ForeignVehicleLost      => if de { "Fremdfahrzeug verloren" }               else { "Foreign vehicle lost" },
            Self::TrackingDurationWarning => if de { "Tracking-Warnung: Zeitlimit erreicht" } else { "Tracking warning: duration limit reached" },
            Self::TrackingDistanceWarning => if de { "Tracking-Warnung: Distanzlimit erreicht" } else { "Tracking warning: distance limit reached" },
            Self::Test                    => if de { "Hallo, ich bin dein virtueller Beifahrer" }                  else { "Hello, i am your virtual passenger" },
        };
        s.to_owned()
    }
}

// Worker thread
struct TtsRequest {
    text:     String,
    language: Option<String>,
}

static SENDER: OnceLock<mpsc::SyncSender<TtsRequest>> = OnceLock::new();

/// Returns the channel sender, starting the worker thread on first call.
fn sender() -> &'static mpsc::SyncSender<TtsRequest> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("cites-tts".into())
            .spawn(move || worker(rx))
            .expect("failed to spawn TTS worker thread");
        tx
    })
}

/// Long-lived TTS worker. Creates the synthesizer once and processes requests
/// sequentially, waiting for each utterance to finish before starting the next.
fn worker(rx: mpsc::Receiver<TtsRequest>) {
    let mut tts = match Tts::default() {
        Ok(t)  => t,
        Err(e) => { eprintln!("TTS init failed: {e}"); return; }
    };

    for req in rx {
        if let Some(lang) = req.language.as_deref() {
            if let Ok(voices) = tts.voices() {
                if let Some(v) = voices.iter().find(|v| v.language().starts_with(lang)) {
                    let _ = tts.set_voice(v);
                }
            }
        }
        let _ = tts.speak(&req.text, false);
        // Give the backend a moment to start before polling.
        std::thread::sleep(Duration::from_millis(100));
        while tts.is_speaking().unwrap_or(false) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// Public API
/// Enqueues `text` for speech output on the persistent TTS worker thread
///
/// Returns immediately; the worker speaks utterances sequentially.
/// If `language` is `Some`, the first matching BCP 47 voice is selected.
/// Messages are silently dropped when the internal queue is ful.
pub fn speak(text: &str, language: Option<&str>) -> Result<(), TtsError> {
    sender().try_send(TtsRequest {
        text:     text.to_owned(),
        language: language.filter(|l| !l.is_empty()).map(str::to_owned),
    }).ok();
    Ok(())
}

//error type
#[derive(Debug)]
pub enum TtsError {
    Init(tts::Error),
    Speak(tts::Error),
}

impl std::fmt::Display for TtsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TtsError::Init(e)  => write!(f, "TTS initialization failed: {e}"),
            TtsError::Speak(e) => write!(f, "TTS speak failed: {e}"),
        }
    }
}

impl std::error::Error for TtsError {}

/// Converts a colon-separated MAC address into a spoken string suitable for
/// TTS output.
///
/// Each hex byte is split into its two characters, separated by a space.
/// Bytes are separated by a comma-space.  This produces clear, unambiguous
/// pronunciation across German and English TTS engines.
///
/// # Example
/// ```
/// assert_eq!(mac_to_spoken("AA:1B:CC:DD:EE:FF"),
///            "A A, 1 B, C C, D D, E E, F F");
/// ```
pub fn mac_to_spoken(mac: &str) -> String {
    mac.to_uppercase()
        .split(':')
        .map(|byte| {
            byte.chars()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}
