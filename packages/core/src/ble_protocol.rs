/// Largest GATT-notification payload the node will ever emit. Equal to the theoretical BLE 5.0 ATT_MTU(512) minus the 3 B ATT header.
/// The actual per-session chunk size is agreed via [`crate::ble_handshake`] and may be smaller depending on what the client can receive. 
///
/// ATT layer overview and packet-size derivation:
/// <https://software-dl.ti.com/simplelink/esd/simplelink_cc13x2_26x2_sdk/2.40.00.81/exports/docs/ble5stack/ble_user_guide/html/ble-stack-5.x/gatt.html>
///
/// Throughput example showing DLE / MTU trade-offs in practice:
/// <https://github.com/Infineon/mtb-example-btsdk-ble-throughput>
pub const BLE_MAX_CHUNK_SIZE: u16 = 509;

/// fragments payload into BLE-sized chnuks with a 4-byte header per fragment
pub fn fragment_payload(seq_num: u64, payload: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let chunk_sz = mtu - 4;
    let total_frags = (payload.len() + chunk_sz - 1) / chunk_sz;
    let mut fragments = Vec::with_capacity(total_frags);

    let seq_lo16 = (seq_num & 0xFFFF) as u16;
    let seq_hi = (seq_lo16 >> 8) as u8;
    let seq_lo = (seq_lo16 & 0xFF) as u8;

    for i in 0..total_frags {
        let start = i * chunk_sz;
        let end = std::cmp::min(start + chunk_sz, payload.len());

        let mut chunk = Vec::with_capacity(end - start + 4);
        chunk.push(seq_hi);
        chunk.push(seq_lo);
        chunk.push(i as u8);
        chunk.push(total_frags as u8);
        chunk.extend_from_slice(&payload[start..end]);

        fragments.push(chunk);
    }
    fragments
}

/// Packs the PCAP message cleanly without serde overhead.
pub fn pack_pcap_message(seq: u64, ts_ns: u64, data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + data.len());
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(&ts_ns.to_be_bytes());
    buf.extend_from_slice(data);
    buf
}

/// Unpacks a packed PCAP message back into its original parts.
/// Returns `Option<(sequence_number, timestamp_ns, data)>`.
pub fn unpack_pcap_message(packed: &[u8]) -> Option<(u64, u64, &[u8])> {
    if packed.len() < 16 {
        return None;
    }

    let seq = u64::from_be_bytes(packed[0..8].try_into().unwrap());
    let ts_ns = u64::from_be_bytes(packed[8..16].try_into().unwrap());

    Some((seq, ts_ns, &packed[16..]))
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReassemblyStatus {
    Pending,
    /// Reassembly complete. Carries the lower-16-bit fragment-header seq (for quick in-flight tracking) and the full reassembled payload.
    /// Callers should extract the authoritative u64 seq via [`unpack_pcap_message`] after this.
    Complete(u16, Vec<u8>),
    Dropped,
}

pub struct BleReassembler {
    /// Lower 16 bits of the seq_num used as the in-flight message identifier.
    current_seq: Option<u16>,
    expected_frags: usize,
    received_frags: usize,
    fragments: Vec<Option<Vec<u8>>>,
}

impl BleReassembler {
    pub fn new() -> Self {
        Self {
            current_seq: None,
            expected_frags: 0,
            received_frags: 0,
            fragments: Vec::new(),
        }
    }

    /// Process one BLE chunk.  Returns `(past_status, current_status)`.
    ///
    /// `past_status` is non-Pending only when a new message starts before the
    /// previous one was fully received (indicating a drop in the middle of a multi-fragment transfer).
    pub fn process_chunk(&mut self, chunk: &[u8]) -> (ReassemblyStatus, ReassemblyStatus) {
        if chunk.len() < 4 {
            return (ReassemblyStatus::Dropped, ReassemblyStatus::Dropped);
        }

        let seq_hi = chunk[0] as u16;
        let seq_lo = chunk[1] as u16;
        let seq = (seq_hi << 8) | seq_lo;
        let frag_idx = chunk[2] as usize;
        let total_frags = chunk[3] as usize;

        if total_frags == 0 || frag_idx >= total_frags {
            self.reset();
            return (ReassemblyStatus::Dropped, ReassemblyStatus::Dropped);
        }

        let mut past_status = ReassemblyStatus::Pending;

        if self.current_seq != Some(seq) {
            if self.received_frags > 0 && self.received_frags < self.expected_frags {
                past_status = ReassemblyStatus::Dropped;
            }
            self.current_seq = Some(seq);
            self.expected_frags = total_frags;
            self.received_frags = 0;
            self.fragments = vec![None; total_frags];
        }

        if self.fragments[frag_idx].is_none() {
            self.fragments[frag_idx] = Some(chunk[4..].to_vec());
            self.received_frags += 1;
        }

        if self.received_frags == self.expected_frags {
            let parts = std::mem::take(&mut self.fragments);
            let mut buf = Vec::new();
            for f in parts {
                if let Some(data) = f {
                    buf.extend_from_slice(&data);
                } else {
                    self.reset();
                    return (past_status, ReassemblyStatus::Dropped);
                }
            }
            self.reset();
            return (past_status, ReassemblyStatus::Complete(seq, buf));
        }

        (past_status, ReassemblyStatus::Pending)
    }

    fn reset(&mut self) {
        self.current_seq = None;
        self.expected_frags = 0;
        self.received_frags = 0;
        self.fragments.clear();
    }
}

impl Default for BleReassembler {
    fn default() -> Self {
        Self::new()
    }
}

// Wire-format utilities

/// Hex-encodes `data` as a lowercase ASCII string (2 chars per byte).
pub fn hex_encode(data: &[u8]) -> String {
    use std::fmt::Write as FmtWrite;
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Decodes a lowercase hex string into bytes.  Returns `None` on any error.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Wraps a packed PCAP message as a single-fragment BLE packet
///
/// Allows non-BLE transports (e.g. HTTP SSE) to feed messages through `BleReassembler` without fragmentation.
/// The sequence number is taken form the lower 16 bits of the 8-byte big-endian sequence field at offset 0.
pub fn to_single_fragment(packed: &[u8]) -> Option<Vec<u8>> {
    if packed.len() < 16 {
        return None; // minimum: seq(8) + ts_ns(8)
    }
    let seq_lo16 = u16::from_be_bytes([packed[6], packed[7]]);
    let mut frag = Vec::with_capacity(4 + packed.len());
    frag.push((seq_lo16 >> 8) as u8);
    frag.push((seq_lo16 & 0xFF) as u8);
    frag.push(0u8); // frag_idx = 0
    frag.push(1u8); // total_frags = 1
    frag.extend_from_slice(packed);
    Some(frag)
}
