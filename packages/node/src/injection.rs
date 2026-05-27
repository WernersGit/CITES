use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::thread;
use tracing::{info, warn, error};
use pcap::Capture;
use core_logic::config::{
    InjectionConfig, InjectionEngineState, InjectionStatus,
    RepeatMode, ReplayProtocol,
};
use core_logic::pcap_parser::{ParsedPacket, PcapParser};
use core_logic::mac_to_station_id;
use core_logic::crc32::crc32;
use crate::capture::{InjectionFrameFilter, body_for_hash};

pub type InjectionStatusHandle = Arc<Mutex<InjectionStatus>>;

/// Handle for controlling and observing a single active injection run
///
/// A new `ActiveInjection` is created for each `start_injection` call.
/// Dropping the handle does not stop the background thread; call [`stop`] explicitly before replacement.
pub struct ActiveInjection {
    pub status: InjectionStatusHandle,
    stop_flag:   Arc<AtomicBool>,
    pause_flag:  Arc<AtomicBool>,
    filter_flag: Arc<AtomicBool>,
}

impl ActiveInjection {
    fn new(filter_inj: bool) -> Self {
        Self {
            status:      Arc::new(Mutex::new(InjectionStatus {
                filter_inj,
                ..Default::default()
            })),
            stop_flag:   Arc::new(AtomicBool::new(false)),
            pause_flag:  Arc::new(AtomicBool::new(false)),
            filter_flag: Arc::new(AtomicBool::new(filter_inj)),
        }
    }

    /// Signals the background thread to terminate after the current packet
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// Toggles pause state.  The background thread will resume on the next `pause_flag` check (<= 50 ms latency).
    pub fn toggle_pause(&self) {
        let was_paused = self.pause_flag.fetch_xor(true, Ordering::Relaxed);
        if let Ok(mut s) = self.status.lock() {
            s.state = if was_paused {
                InjectionEngineState::Running
            } else {
                InjectionEngineState::Paused
            };
        }
    }

    /// Live-update the capture-filter toggle. Takes effect from the next packet onwards; reflected immediately in `status.filter_inj`.
    pub fn set_filter(&self, on: bool) {
        self.filter_flag.store(on, Ordering::Relaxed);
        if let Ok(mut s) = self.status.lock() {
            s.filter_inj = on;
        }
    }
}

/// Starts a new injection run in a background thread and returns a handle.
///
/// `inj_filter` is the shared capture-filter set; each frame's hash is inserted before `sendpacket` so the capture thread can drop the loopback copy.
/// Pass `None` to skip filter population (e.g. dry-run or tests).
pub fn start_injection(
    config:     InjectionConfig,
    interface:  String,
    inj_filter: Option<InjectionFrameFilter>,
) -> ActiveInjection {
    let active = ActiveInjection::new(config.filter_inj);

    let status      = Arc::clone(&active.status);
    let stop_flag   = Arc::clone(&active.stop_flag);
    let pause_flag  = Arc::clone(&active.pause_flag);
    let filter_flag = Arc::clone(&active.filter_flag);

    thread::spawn(move || {
        run_injection(config, interface, inj_filter, status, stop_flag, pause_flag, filter_flag);
    });

    active
}

// private implementation

fn run_injection(
    config:      InjectionConfig,
    interface:   String,
    inj_filter:  Option<InjectionFrameFilter>,
    status:      InjectionStatusHandle,
    stop_flag:   Arc<AtomicBool>,
    pause_flag:  Arc<AtomicBool>,
    filter_flag: Arc<AtomicBool>,
) {
    set_tx_power(&interface, config.tx_power_dbm);

    // open the raw interface for injection unless this is a dry run
    let mut sender = if config.dry_run {
        info!("InjectionEngine: dry-run mode - no packets will be sent");
        None // dry run, nothing to do
    } else {
        match Capture::from_device(interface.as_str()) {
            Ok(c) => match c.immediate_mode(true).open() {
                Ok(cap) => Some(cap),
                Err(e) => {
                    error!("InjectionEngine: failed to open '{interface}' for injection: {e}");
                    set_state(&status, InjectionEngineState::Error(e.to_string()));
                    return;
                }
            },
            Err(e) => {
                error!("InjectionEngine: no device '{}': {}", interface, e);
                set_state(&status, InjectionEngineState::Error(e.to_string()));
                return;
            }
        }
    };

    // quick sanity check —> reject anything that looks like a path
    if config.archive_filename.contains('/') || config.archive_filename.contains("..") {
        set_state(&status, InjectionEngineState::Error("invalid filename".into()));
        return;
    }

    let path = Path::new("./captures").join(&config.archive_filename);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            error!("InjectionEngine: cannot read '{}': {}", config.archive_filename, e);
            set_state(&status, InjectionEngineState::Error(format!("couldn't read file: {e}")));
            return;
        }
    };

    let pkts = match PcapParser::parse_bytes_raw(&bytes) {
        Ok(pkts) => pkts,
        Err(e) => {
            error!("InjectionEngine: cannot parse '{}': {}", config.archive_filename, e);
            set_state(&status, InjectionEngineState::Error(format!("parse failed: {e}")));
            return;
        }
    };

    let base_ts = pkts
        .first()
        .map(|(p, _)| p.timestamp_ms)
        .unwrap_or(0);

    let packets: Vec<(ParsedPacket, Vec<u8>)> = pkts
        .into_iter()
        .filter(|(pkt, _)| packet_matches(&pkt, &config, base_ts))
        .collect();

    let total = packets.len() as u64;
    info!(
        "InjectionEngine: {} pakects after filtering from '{}'",
        total, config.archive_filename
    );

    {
        let mut s = status.lock().unwrap();
        s.packets_total = total;
        s.state = InjectionEngineState::Running;
    }

    if total == 0 {
        error!("InjectionEngine: 0 packets after filtering in '{}'", config.archive_filename);
        set_state(&status, InjectionEngineState::Error(
            "No packets found — archive is empty or contains no Car2X frames.".into()
        ));
        return;
    }

    let max_iter: Option<u32> = match &config.schedule.repeat {
        RepeatMode::Once     => Some(1),
        RepeatMode::Count(n) => Some(*n),
        RepeatMode::Infinite => None,
    };

    // use the first filtered packet's timestamp as the reference so preserve_timing
    // reflects inter-packet gaps only, not the gap between archive start and first CAM

    let ref_ts = packets.first().map(|(p, _)| p.timestamp_ms).unwrap_or(base_ts);

    let t0    = Instant::now();
    let mut sent      = 0u64;
    let mut failed    = 0u64;
    let mut iteration = 0u32;

    'outer: loop {
        if stop_flag.load(Ordering::Relaxed) { break; }

        let mut prev_ts = ref_ts;

        for (idx, (pkt, raw)) in packets.iter().enumerate() {
            // pause: spin with short sleeps until resumed or stopped
            while pause_flag.load(Ordering::Relaxed) {
                if stop_flag.load(Ordering::Relaxed) { break 'outer; }
                thread::sleep(Duration::from_millis(50));
            }
            if stop_flag.load(Ordering::Relaxed) { break 'outer; }

            let delay_ms = if config.schedule.preserve_timing {
                (pkt.timestamp_ms - prev_ts).max(0) as u64
            } else {
                config.schedule.packet_delay_ms
            };
            let delay = with_jitter(delay_ms, config.schedule.jitter_ms, idx);
            if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
            }
            prev_ts = pkt.timestamp_ms;

            if let Some(ref mut cap) = sender {
                // replace the captured RX Radiotap header with a minimal TX header so the driver accepts the frame on the injection path
                let frame = build_inject_frame(raw);
                // pre-register the body-hash so the capture thread can drop the loopback copy; the flag is live-updatable
                let hash = filter_flag
                    .load(Ordering::Relaxed)
                    .then(|| crc32(body_for_hash(&frame)));
                if let (Some(h), Some(f)) = (hash, inj_filter.as_ref()) {
                    f.lock().unwrap().insert(h);
                }
                if cap.sendpacket(frame.as_slice()).is_ok() {
                    sent += 1;
                } else {
                    // sendpacket failed -> roll back the pre-registered hash
                    if let (Some(h), Some(f)) = (hash, inj_filter.as_ref()) {
                        f.lock().unwrap().remove(&h);
                    }
                    failed += 1;
                    if failed == 1 || failed % 100 == 0 {
                        error!("InjectionEngine: sendpacket failed ({} total); \
                                check that '{}' supports injection and \
                                that a TX Radiotap header is accepted by the driver",
                            failed, interface);
                    }
                }
            } else {
                sent += 1;
            }

            // update status every 10 packets or every second (whichever comes first) so BLE polling at 1.5 s always sees fresh progress
            let elapsed = t0.elapsed().as_millis() as u64;
            let tick = elapsed.saturating_sub(
                if let Ok(s) = status.try_lock() { s.elapsed_ms } else { elapsed }
            ) >= 1000;
            if sent % 10 == 0 || tick || idx + 1 == packets.len() {
                if let Ok(mut s) = status.lock() {
                    s.packets_sent    = sent;
                    s.current_iteration = iteration + 1;
                    s.elapsed_ms      = t0.elapsed().as_millis() as u64;
                }
            }
        }

        iteration += 1;
        if let Some(max) = max_iter {
            if iteration >= max { break; }
        }

        // inter-loop delay (can be interrupted by stop)
        if config.schedule.loop_delay_ms > 0 {
            let deadline = Instant::now() + Duration::from_millis(config.schedule.loop_delay_ms);
            while Instant::now() < deadline {
                if stop_flag.load(Ordering::Relaxed) { break 'outer; }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    let end_state = if stop_flag.load(Ordering::Relaxed) {
        InjectionEngineState::Idle
    } else {
        InjectionEngineState::Completed
    };

    if let Ok(mut s) = status.lock() {
        s.state           = end_state;
        s.packets_sent    = sent;
        s.elapsed_ms      = t0.elapsed().as_millis() as u64;
    }

    //drop any remaining loopback FCS entries so they cannot match unrelated traffic after this run ends
    if let Some(ref f) = inj_filter {
        if let Ok(mut set) = f.lock() {
            set.clear();
        }
    }

    info!("InjectionEngine: finished — {} packets sent. ", sent);
}

fn packet_matches(pkt: &ParsedPacket, config: &InjectionConfig, base_ts: i64) -> bool {
    let f = &config.filter;

    if let Some(vid) = f.vehicle_id {
        if mac_to_station_id(&pkt.mac) != vid { return false; }
    }

    let offset_ms = pkt.timestamp_ms.saturating_sub(base_ts) as u64;
    if let Some(start) = f.time_range_start_ms {
        if offset_ms < start { return false; }
    }
    if let Some(end) = f.time_range_end_ms {
        if offset_ms > end { return false; }
    }

    if !f.protocols.is_empty() {
        match pkt.btp_b_info.as_ref().and_then(|b| port_to_protocol(b.destination_port)) {
            Some(p) if f.protocols.contains(&p) => {}
            _ => return false,
        }
    }

    true
}

fn port_to_protocol(port: u16) -> Option<ReplayProtocol> {
    match port {
        2001 => Some(ReplayProtocol::Cam),
        2002 => Some(ReplayProtocol::Denm),
        2003 => Some(ReplayProtocol::Mapem),
        2004 => Some(ReplayProtocol::Spatem),
        2006 => Some(ReplayProtocol::Ivim),
        2007 => Some(ReplayProtocol::Srem),
        2008 => Some(ReplayProtocol::Ssem),
        2009 => Some(ReplayProtocol::Cpm),
        _    => None,
    }
}

// TODO: replace with proper RNG at some point
fn with_jitter(base: u64, jitter: u64, index: usize) -> u64 {
    if jitter == 0 { return base; }
    let n = (index as u64)
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
        >> 33;
    base + (n % (jitter + 1))
}

fn set_state(handle: &InjectionStatusHandle, state: InjectionEngineState) {
    if let Ok(mut s) = handle.lock() {
        s.state = state;
    }
}

/// Sets the transmit power on `interface` via `iw dev <iface> set txpower fixed <mBm>`.
/// Does nothing when `power_dbm` is `None`.
fn set_tx_power(interface: &str, power_dbm: Option<u8>) {
    let Some(dbm) = power_dbm else { return };
    let mbm = (dbm as i32) * 100;
    match std::process::Command::new("iw")
        .args(["dev", interface, "set", "txpower", "fixed", &mbm.to_string()])
        .status()
    {
        Ok(s) if s.success() => info!("InjectionEngine: TX power set to {dbm} dBm"),
        Ok(s) => warn!("InjectionEngine: iw txpower exited with {:?}", s.code()),
        Err(e) => warn!("InjectionEngine: iw command failed: {}", e),
    }
}

/// Strips the RX Radiotap header from a captured frame and returns the 802.11 body.
fn strip_radiotap(frame: &[u8]) -> &[u8] {
    if frame.len() < 4 || frame[0] != 0 {
        return frame;
    }
    let rt_len = u16::from_le_bytes([frame[2], frame[3]]) as usize;
    if rt_len >= frame.len() {
        return frame;
    }
    &frame[rt_len..]
}

/// Replaces the captured RX Radiotap header with a minimal TX Radiotap header accepted by the ath9k injection path, then appends the 802.11 frame body.
///
/// TX Radiotap layout (12 bytes):
/// - version=0, pad=0, len=12 (LE u16)
/// - present = RATE(bit 2) | TX_FLAGS(bit 15) → 0x0000_8004 (LE u32)
/// - Rate = 12  (6 Mbps at 500 kbps/unit)
/// - 1 alignment pad byte
/// - TX_FLAGS = 0x0008 (IEEE80211_RADIOTAP_F_TX_NOACK, LE u16)
fn build_inject_frame(frame: &[u8]) -> Vec<u8> {
    let body = strip_radiotap(frame);
    let mut out = Vec::with_capacity(12 + body.len());
    out.push(0u8);                                // revision
    out.push(0u8);                                // pad
    out.extend_from_slice(&12u16.to_le_bytes());  // header length
    out.extend_from_slice(&0x0000_8004u32.to_le_bytes()); // present: RATE + TX_FLAGS
    out.push(12u8);                               // rate: 12 * 500 kbps = 6 Mbps
    out.push(0u8);                                // alignment pad for TX_FLAGS
    out.extend_from_slice(&0x0008u16.to_le_bytes()); // TX_FLAGS: NOACK
    out.extend_from_slice(body);
    out
}
