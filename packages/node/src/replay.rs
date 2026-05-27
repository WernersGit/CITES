use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;
use crossbeam_channel::Receiver;
use tracing::{info, warn, error, debug};
use pcap::Capture;
use core_logic::config::{RepeatModeConfig, ReplayProtocol};
use core_logic::crc32::crc32;
use core_logic::pcap_parser::{PcapParser, ParsedPacket};
use core_logic::mac_to_station_id;
use crate::capture::{InjectionFrameFilter, PcapMessage, body_for_hash};

/// Shared handle for updating the replay config from HTTP handlers.
pub type ReplayConfigHandle = Arc<Mutex<RepeatModeConfig>>;

/// Shared handle exposing the cumulative count of successfully replayed packets.
/// Written by the engine thread, readable from HTTP handlers without locking.
pub type ReplayCountHandle = Arc<AtomicU64>;

/// Receives captured packets from the dispatcher, applies the active
/// [`RepeatModeConfig`] filters, and immediately re-injects matching frames
/// back onto the network interface.
///
/// # Placement in the pipeline
///
/// ```text
/// CaptureDispatcher
///   ├── unbounded -> archive writer  (lossless)
///   ├── bounded   -> BLE ring buffer (lossy)
///   └── bounded   -> ReplayEngine   (lossy, small buffer for low latency)
/// ```
///
/// The engine runs in its own thread and does nothing while
/// `RepeatModeConfig::enabled` is `false`, so it has zero processing cost
/// when replay is inactive.
pub struct ReplayEngine {
    config: ReplayConfigHandle,
    replay_count: ReplayCountHandle,
}

impl ReplayEngine {
    pub fn new() -> Self {
        Self {
            config: Arc::new(Mutex::new(RepeatModeConfig::default())),
            replay_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns a cloneable handle for updating the config at runtime
    pub fn config_handle(&self) -> ReplayConfigHandle {
        Arc::clone(&self.config)
    }

    /// Returns a cloneable handle for reading the cumulative replay counter
    pub fn replay_count_handle(&self) -> ReplayCountHandle {
        Arc::clone(&self.replay_count)
    }

    /// spawns the worker thread
    ///
    /// `inj_filter` is the shared capture filter; the engine inserts the FCS of every replayed frame before `sendpacket` 
    /// and clears the set when replay turns off, so loopback copies never reach the live broadcast.
    pub fn start(
        self,
        rx: Receiver<PcapMessage>,
        interface: String,
        inj_filter: InjectionFrameFilter,
    ) {
        let config = self.config;
        let replay_count = self.replay_count;

        let iface = interface;
        thread::spawn(move || {
            // open a separate handle on the same interface for injection
            let mut sender = match Capture::from_device(iface.as_str()) {
                Ok(c) => match c.immediate_mode(true).open() {
                    Ok(opened) => opened,
                    Err(e) => {
                        error!("ReplayEngine: cannot open '{iface}' for injection: {e}");
                        return;
                    }
                },
                Err(e) => {
                    error!("ReplayEngine: interface '{}' not available: {}", iface, e);
                    return;
                }
            };

            info!("ReplayEngine: ready on interfcae '{iface}'");

            // cache last power setting to skip redundant iw calls
            let mut cur_pwr: Option<Option<u8>> = None;
            // tracks the engine's previous enabled state so we can flush the capture filter exactly once on the on-to-off transition
            let mut was_enabled = false;

            while let Ok(msg) = rx.recv() {
                let cfg = match config.lock() {
                    Ok(g) => g.clone(),
                    Err(_) => continue,
                };

                if was_enabled && !cfg.enabled {
                    if let Ok(mut set) = inj_filter.lock() {
                        set.clear();
                    }
                }
                was_enabled = cfg.enabled;

                // apply TX power whenever it changes
                if cur_pwr != Some(cfg.tx_power_dbm) {
                    cur_pwr = Some(cfg.tx_power_dbm);
                    if let Some(dbm) = cfg.tx_power_dbm {
                        let mbm = (dbm as i32) * 100;
                        match std::process::Command::new("iw")
                            .args(["dev", iface.as_str(), "set", "txpower", "fixed",
                                   &mbm.to_string()])
                            .status()
                        {
                            Ok(s) if s.success() =>
                                info!("ReplayEngine: TX power set to {} dBm", dbm),
                            Ok(s) =>
                                warn!("ReplayEngine: iw txpower exited with {:?}", s.code()),
                            Err(e) =>
                                warn!("ReplayEngine: iw command failed: {e}"),
                        }
                    }
                }

                if !cfg.enabled {
                    continue;
                }

                // parse enough to apply filters if the packet cannot be decoded (e.g. malformed radiotap), skip it silently
                let parsed = match PcapParser::parse_live_packet(msg.timestamp_ns, &msg.data) {
                    Some(p) => p,
                    None => continue,
                };

                if !Self::matches_vehicle(&parsed, &cfg) {
                    continue;
                }

                if !Self::matches_protocol(&parsed, &cfg) {
                    continue;
                }

                // pre-register the loopback body-hash so the capture thread can drop the copy. `body_for_hash` strips both the Radiotap header and any FCS the driver appends, so both sides agree
                let hash = crc32(body_for_hash(&msg.data));
                inj_filter.lock().unwrap().insert(hash);

                match sender.sendpacket(msg.data.as_slice()) {
                    Ok(_) => {
                        let count = replay_count.fetch_add(1, Ordering::Relaxed) + 1;
                        debug!(
                            seq = msg.sequence_number,
                            mac = %parsed.mac,
                            port = ?parsed.btp_b_info.as_ref().map(|b| b.destination_port),
                            count,
                            "ReplayEngine: replayed packet"
                        );

                        if cfg.delay_ms > 0 {
                            thread::sleep(Duration::from_millis(cfg.delay_ms));
                        }
                    }
                    Err(e) => {
                        // sendpacket failed -> roll back the pre-registered hash
                        inj_filter.lock().unwrap().remove(&hash);
                        warn!(
                            seq = msg.sequence_number,
                            "ReplayEngine: sendpacket failed: {}",
                            e
                        );
                    }
                }
            }

            if let Ok(mut set) = inj_filter.lock() {
                set.clear();
            }
            info!("ReplayEngine: chanell closed, shutting down.");
        });
    }

    // filter helpers

    /// Returns `true` when the packet's source vehicle matches the configured vehicle filter
    ///
    /// The vehicle ID is currently derived from the 802.11 transmitter MAC address (bytes 2–5 interpreted as a big-endian u32). 
    /// This is a stable per-vehicle proxy until full ASN.1 decoding of the ITS StationID field is available.
    fn matches_vehicle(parsed: &ParsedPacket, cfg: &RepeatModeConfig) -> bool {
        match cfg.vehicle_id_filter {
            None => true,
            Some(target_id) => mac_to_station_id(&parsed.mac) == target_id,
        }
    }

    /// Returns `true` when the packet's BTP destination port matches one of the configured protocols (or when no protocol filter is set).
    fn matches_protocol(parsed: &ParsedPacket, cfg: &RepeatModeConfig) -> bool {
        if cfg.protocol_filter.is_empty() {
            return true;
        }

        let Some(btp) = &parsed.btp_b_info else {
            // cannot determine protocol -> do not replay when a filter is set
            return false;
        };

        let proto = match btp.destination_port {
            2001 => ReplayProtocol::Cam,
            2002 => ReplayProtocol::Denm,
            2003 => ReplayProtocol::Mapem,
            2004 => ReplayProtocol::Spatem,
            2006 => ReplayProtocol::Ivim,
            2007 => ReplayProtocol::Srem,
            2008 => ReplayProtocol::Ssem,
            2009 => ReplayProtocol::Cpm,
            _ => return false,
        };

        cfg.protocol_filter.contains(&proto)
    }
}

