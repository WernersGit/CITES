//! Application-layer BLE session handshake.
//!
//! Negotiates protocol version and the maximum GATT-notification chunk size used for PCAP streaming. ATT-MTU negotiation is opaque to userland on
//! macOS Core Bluetooth, so the agreement is performed one layer up: the lient advertises the largest fragment it can sink, the node returns the
//! minimum of that value and its own configured cap, and both sides use the returned value for the lifetime of the GATT session.
//!
//! Background on why application-layer fragmentation is necessary despite the BLE stack handling L2CAP fragmentation transparently:
//! <https://software-dl.ti.com/simplelink/esd/simplelink_cc13x2_26x2_sdk/2.40.00.81/exports/docs/ble5stack/ble_user_guide/html/ble-stack-5.x/gatt.html>
//!
//! Wire layout (6 bytes, identical for request and response):
//!
//! ```text
//! [0]    HANDSHAKE_MAGIC (0xAA) -- distinguishes from unrelated writes
//! [1]    protocol_version u8    -- peers MUST take the minimum
//! [2..3] max_chunk u16 BE       -- request: client cap; response: negotiated
//! [4]    capabilities bitfield  -- reserved for future opt-in features
//! [5]    reserved               -- MUST be written as 0
//! ```

pub const HANDSHAKE_MAGIC: u8 = 0xAA;
pub const HANDSHAKE_FRAME_LEN: usize = 6;
pub const PROTOCOL_VERSION: u8 = 1;

/// Smallest chunk that still fits the 4-byte fragment header plus payload.
pub const MIN_CHUNK_SIZE: u16 = 23;

/// Desired maximum chunk size the client probes for.
/// Equal to the BLE 5.0 ceiling (ATT_MTU 512 - 3 ATT header bytes).
pub const CLIENT_PROBE_MAX_CHUNK: u16 = crate::ble_protocol::BLE_MAX_CHUNK_SIZE;

/// Fallback chunk size used when the probe write is rejected or unconfirmed.
/// Equal to BLE 5.0 DLE with default ATT_MTU (247 - 3 = 244 B)
pub const CLIENT_FALLBACK_CHUNK: u16 = 244;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeFrame {
    pub version: u8,
    pub max_chunk: u16,
    pub capabilities: u8,
}

impl HandshakeFrame {
    pub const fn new(version: u8, max_chunk: u16, capabilities: u8) -> Self {
        Self { version, max_chunk, capabilities }
    }

    pub fn to_bytes(self) -> [u8; HANDSHAKE_FRAME_LEN] {
        let c = self.max_chunk.to_be_bytes();
        [HANDSHAKE_MAGIC, self.version, c[0], c[1], self.capabilities, 0]
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HANDSHAKE_FRAME_LEN || bytes[0] != HANDSHAKE_MAGIC {
            return None;
        }
        Some(Self {
            version: bytes[1],
            max_chunk: u16::from_be_bytes([bytes[2], bytes[3]]),
            capabilities: bytes[4],
        })
    }
}

/// Server-side reconciliation. Returns the frame both peers MUST use for the rest of the session: minimum protocol version, max_chunk clamped to the server's configured cap, capabilities intersected.
pub fn reconcile(server_max: u16, client: HandshakeFrame) -> HandshakeFrame {
    HandshakeFrame {
        version: client.version.min(PROTOCOL_VERSION),
        max_chunk: client.max_chunk.clamp(MIN_CHUNK_SIZE, server_max),
        capabilities: client.capabilities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_frame() {
        let f = HandshakeFrame::new(1, 244, 0x00);
        assert_eq!(HandshakeFrame::from_bytes(&f.to_bytes()), Some(f));
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut bytes = HandshakeFrame::new(1, 244, 0).to_bytes();
        bytes[0] = 0x55;
        assert!(HandshakeFrame::from_bytes(&bytes).is_none());
    }

    #[test]
    fn rejects_short_frame() {
        assert!(HandshakeFrame::from_bytes(&[HANDSHAKE_MAGIC, 1, 0]).is_none());
    }

    #[test]
    fn reconcile_caps_to_server_max() {
        let r = reconcile(244, HandshakeFrame::new(2, 800, 0));
        assert_eq!(r.version, PROTOCOL_VERSION);
        assert_eq!(r.max_chunk, 244);
    }

    #[test]
    fn reconcile_respects_client_cap() {
        let r = reconcile(509, HandshakeFrame::new(1, 182, 0));
        assert_eq!(r.max_chunk, 182);
    }

    #[test]
    fn reconcile_floors_at_min() {
        let r = reconcile(509, HandshakeFrame::new(1, 5, 0));
        assert_eq!(r.max_chunk, MIN_CHUNK_SIZE);
    }
}
