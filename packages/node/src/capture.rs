use crossbeam_channel::{bounded, unbounded, Receiver};
use pcap::Capture;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{info, error};
use api::storage::{PcapStorageManager, StorageMode};

/// Shared filter populated by the injection or replay engine before each
/// `sendpacket`. The capture thread removes matching FCS values on first hit
/// (one-time suppression). Producers must `clear()` the set when they stop so
/// no stale entries match unrelated traffic later.
pub type InjectionFrameFilter = Arc<Mutex<HashSet<u32>>>;

#[derive(Debug, Clone)]
pub struct PcapMessage {
    pub sequence_number: u64,
    pub timestamp_ns: u64,
    pub data: Vec<u8>,
}

/// Receivers handed back to the caller after the dispatcher starts.
pub struct DispatcherChannels {
    /// Feed to the live broadcast fan-out task.  Always present.
    pub live_rx: Receiver<PcapMessage>,
    /// Feed to the `ReplayEngine`.  Always present; the engine ignores
    /// packets when its config has `enabled = false`.
    pub replay_rx: Receiver<PcapMessage>,
}

pub struct CaptureDispatcher {
    interface_name: String,
    /// Frames queued here by the injection engine are silently dropped by the
    /// capture thread so that locally-transmitted packets are not archived.
    pub injection_filter: InjectionFrameFilter,
}

impl CaptureDispatcher {
    pub fn new(interface_name: String) -> Self {
        Self {
            interface_name,
            injection_filter: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// starts capture + storage threads, returns channel endpoints
    pub fn start(self) -> DispatcherChannels {
        let (archive_tx, arch_rx) = unbounded::<PcapMessage>();

        // TODO: maybe expose buffer size as config
        // live ring-buffer channel (lossy, always present)
        let (live_tx, live_rx) = bounded::<PcapMessage>(250);

        let live_drop = live_rx.clone();

        // replay channel - small buffer so the engine stays as close to
        // real-time as possible; old packets are dropped on overflow
        let (replay_tx, replay_rx) = bounded::<PcapMessage>(64);
        let mut drop_rx = replay_rx.clone(); // used to evict oldest on overflow

        // archive thread (lossless)
        thread::spawn(move || {
            let mut storage = match PcapStorageManager::new(StorageMode::Archive, "./captures") {
                Ok(s) => s,
                Err(e) => {
                    error!("archive storage init failed: {e}");
                    return;
                }
            };

            info!("Archvie storage initialized: {:?}", storage.filepath);

            while let Ok(msg) = arch_rx.recv() {
                if let Err(e) = storage.write_packet(msg.timestamp_ns, &msg.data) {
                    error!("Failed to write packet to archive: {}", e);
                }
            }

            info!("Archive thread closing.");
            if let Err(e) = storage.close() {
                error!("Error closing archive storage: {e}");
            }
        });

        let iface = self.interface_name.clone();
        let inj_filter = Arc::clone(&self.injection_filter);

        // capture thread
        thread::spawn(move || {
            let mut cap = match Capture::from_device(iface.as_str()) {
                Ok(c) => match c.promisc(true).immediate_mode(true).open() {
                    Ok(opened) => opened,
                    Err(e) => {
                        error!("couldn't open capture device: {}", e);
                        return;
                    }
                },
                Err(e) => {
                    error!("capture device '{iface}' not found: {e}");
                    return;
                }
            };

            let datalink = cap.get_datalink();
            info!("Capture started on interface: {} (datalink {:?}/{})",
                iface, datalink, datalink.0);

            let mut seq: u64 = 0;
            let mut live_tx_opt = Some(live_tx);

            while let Ok(packet) = cap.next_packet() {
                // skip frames injected or replayed by the local node
                let body = body_for_hash(packet.data);
                if !body.is_empty() {
                    let h = core_logic::crc32::crc32(body);
                    if inj_filter.lock().unwrap().remove(&h) {
                        continue;
                    }
                }

                seq = seq.wrapping_add(1);

                let sec = packet.header.ts.tv_sec as u64;
                let usec = packet.header.ts.tv_usec as u64;
                let ts = (sec * 1_000_000_000) + (usec * 1_000);

                let msg = PcapMessage {
                    sequence_number: seq,
                    timestamp_ns: ts,
                    data: packet.data.to_vec(),
                };

                // 1. unbounded channel -> local archive (lossless)
                if let Err(e) = archive_tx.send(msg.clone()) {
                    error!("Archive channel closed: {}", e);
                    break;
                }

                // 2. bounded channel -> live broadcast ring buffer (lossy)
                if let Some(ref tx) = live_tx_opt {
                    match tx.try_send(msg.clone()) {
                        Ok(_) => {}
                        Err(crossbeam_channel::TrySendError::Full(_)) => {
                            let _ = live_drop.try_recv();
                            let _ = tx.try_send(msg.clone());
                        }
                        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                            live_tx_opt = None;
                        }
                    }
                }

                // 3. bounded channel -> replay engine (lossy, low-latency ring buffer)
                match replay_tx.try_send(msg.clone()) {
                    Ok(_) => {}
                    Err(crossbeam_channel::TrySendError::Full(_)) => {
                        // drop the oldest queued packet to make room for the new one
                        let _ = drop_rx.try_recv();
                        let _ = replay_tx.try_send(msg);
                    }
                    Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                        // ReplayEngine shut down - stop feeding the channel
                        break;
                    }
                }
            }
        });

        DispatcherChannels {
            live_rx,
            replay_rx,
        }
    }
}

/// Returns the byte offset where the 802.11 body starts inside a frame carrying a Radiotap header (datalink `IEEE80211_RADIO`).
///
/// Yields `0` when the header is missing or malformed, leaving callers free to operate on the full slice in that case.
pub fn radiotap_len(frame: &[u8]) -> usize {
    if frame.len() < 4 || frame[0] != 0 { return 0; }
    let rt_len = u16::from_le_bytes([frame[2], frame[3]]) as usize;
    if rt_len >= frame.len() { 0 } else { rt_len }
}

/// Returns the 802.11 body slice used for loopback de-duplication hashing.
///
/// Strips the Radiotap header at the start and, when Radiotap signals `IEEE80211_RADIOTAP_F_FCS`, the 4-byte FCS at the end. Whether the FCS is appended depends on the driver and the kind of TX path (real RF vs.
/// Monitor-mode loopback); checking the flag means inject and capture sides hash exactly the same bytes regardless.
pub fn body_for_hash(frame: &[u8]) -> &[u8] {
    let start = radiotap_len(frame);
    let end = if radiotap_has_fcs(frame) {
        frame.len().saturating_sub(4)
    } else {
        frame.len()
    };
    if start >= end { return &[]; }
    &frame[start..end]
}

/// Walks the Radiotap present-bitmap chain to find the FLAGS field and tests its `IEEE80211_RADIOTAP_F_FCS` bit (0x10). 
/// Conservatively returns `false` for any short or malformed header.
fn radiotap_has_fcs(frame: &[u8]) -> bool {
    if frame.len() < 8 || frame[0] != 0 { return false; }
    let rt_len = u16::from_le_bytes([frame[2], frame[3]]) as usize;
    if rt_len < 8 || rt_len > frame.len() { return false; }

    // first present bitmap at offset 4; bit 31 means another bitmap follows
    let primary = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
    let mut present = primary;
    let mut bitmap_end = 8;
    while present & (1 << 31) != 0 && bitmap_end + 4 <= rt_len {
        present = u32::from_le_bytes([
            frame[bitmap_end], frame[bitmap_end + 1],
            frame[bitmap_end + 2], frame[bitmap_end + 3],
        ]);
        bitmap_end += 4;
    }

    if primary & 0x02 == 0 { return false; } // FLAGS field not present

    let mut off = bitmap_end;
    if primary & 0x01 != 0 {
        // TSFT: 8 bytes, align 8
        off = (off + 7) & !7;
        off += 8;
    }
    // FLAGS: 1 byte, no alignment
    if off >= rt_len { return false; }
    frame[off] & 0x10 != 0
}
