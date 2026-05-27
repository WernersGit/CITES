use radiotap::Radiotap;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;
use pcap_file::pcapng::{PcapNgReader, Block};

use crate::RssiMeasurement;

// GNW Long Position Vector constants

/// Length of the GNW Common Header in bytes
const GNW_COMMON_HDR_LEN: usize = 8;
/// Byte offset of the TST field within an LPV (after GN_ADDR = 8 bytes)
const LPV_TST_OFFSET: usize = 8;
/// Byte offset of Latitude within an LPV (after GN_ADDR(8) + TST(4))
const LPV_LAT_OFFSET: usize = 12;

/// Returns the byte offset of the Source Long Position Vector from the start of
/// the Extended Header for the given Header Type (`ht`) and Sub-type (`hst`).
///
/// Offsets
/// - Beacon (1): ext[0]
/// - GUC    (2): ext[12]  — SN(2)+Res(2)+DestAddr(8)
/// - GAC    (3): ext[4]   — SN(2)+Res(2)
/// - GBC    (4): ext[4]   — SN(2)+Res(2)
/// - SHB  (5,0): ext[0]
/// - TSB  (5,_): ext[4]   — SN(2)+Res(2)
fn lpv_ext_offset(ht: u8, hst: u8) -> Option<usize> {
    match (ht, hst) {
        (1, _) => Some(0),
        (2, _) => Some(12),
        (3, _) | (4, _) => Some(4),
        (5, 0) => Some(0),
        (5, _) => Some(4),
        _ => None,
    }
}

/// Represents extracted GeoNetworking data from C-ITS messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GnwInfo {
    pub latitude: i32,
    pub longitude: i32,
    pub speed: u16,
    pub heading: u16,
    /// TST field from the Source Long Position Vector:
    /// TAI milliseconds since 2004-01-01 00:00:00 UTC, wraps at 2^32 ms (≈ 49.7 days).
    pub gnw_timestamp_ms: Option<u32>,
}

/// Represents extracted BTP-B information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BtpBInfo {
    pub destination_port: u16,
    pub destination_port_info: u16,
}

/// Represents a parsed packet containing extracted values from multiple network layers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedPacket {
    pub timestamp_ms: i64,
    pub mac: String,
    pub rssi: f64,
    pub data_len: usize,
    pub gnw_info: Option<GnwInfo>,
    pub btp_b_info: Option<BtpBInfo>,
    #[serde(skip_deserializing)]
    pub payload: Option<crate::parser::decoder::ItsPayload>,
    /// Hardware MAC timestamp from Radiotap TSFT (microseconds)
    pub mactime_us: Option<u64>,
    /// IEEE 802.11 QoS sequence number (bits 4-15 of Sequence Control, 0..4095).
    /// Not reset on pseudonym change; used for hard-constraint pseudonym linking.
    pub frame_seq: Option<u16>,
}

impl Into<RssiMeasurement> for ParsedPacket {
    fn into(self) -> RssiMeasurement {
        RssiMeasurement {
            timestamp_ms: self.timestamp_ms,
            mac: self.mac,
            rssi: self.rssi,
        }
    }
}

/// Modular parser for 802.11p and GeoNetworking packets
pub struct PcapParser;

impl PcapParser {
    /// Parses the Radiotap header
    /// Returns (header_len, rssi, mactime_us)
    pub fn parse_radiotap(data: &[u8]) -> Option<(usize, f64, Option<u64>)> {
        let rt = Radiotap::from_bytes(data).ok()?;
        let header_len = rt.header.length as usize;
        let rssi = match rt.antenna_signal {
            Some(sig) => sig.value as f64,
            None => -100.0,
        };
        let mactime = rt.tsft.map(|t| t.value);
        Some((header_len, rssi, mactime))
    }

    /// Parses the IEEE 802.11 MAC header
    pub fn parse_wlan_mac(data: &[u8]) -> Option<(String, usize)> {
        if data.len() < 24 {
            return None;
        }
        
        let addr = &data[10..16];
        let mac = format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", 
            addr[0], addr[1], addr[2], addr[3], addr[4], addr[5]);

        let fc_type = (data[0] >> 2) & 0b11;
        let fc_subtype = data[0] >> 4;
        let is_qos = fc_type == 2 && (fc_subtype & 0b1000) != 0;
        let header_len = if is_qos { 26 } else { 24 };

        Some((mac, header_len))
    }

    //skip LLC header (always 8 bytes for SNAP)
    pub fn parse_llc(data: &[u8]) -> Option<usize> {
        if data.len() < 8 {
            return None;
        }
        Some(8)
    }

    /// Parses the BTP-B header from a GeoNetworking packet
    ///
    /// Returns `(BtpBInfo, its_payload)` where `its_payload` is the slice of ITS application payload bytes (after the BTP-B header).
    ///
    /// Handles both:
    /// - **NH=1** (unsecured): Common Header follows directly after the Basic Header
    /// - **NH=2** (secured):   IEEE 1609.2 security wrapper is
    ///   stripped first to recover the signed GNW payload (Common Header onwards).
    pub fn parse_btp_b(gnw_data: &[u8]) -> Option<(BtpBInfo, &[u8])> {
        if gnw_data.len() < 12 { return None; }

        let next_hdr = gnw_data[0] & 0x0F;

        let common: &[u8] = match next_hdr {
            // unsecured: Common Header starts right after the 4-byte Basic Header
            1 => &gnw_data[4..],
            // secured: strip the IEEE 1609.2 envelope
            2 => Self::strip_etsi103097(gnw_data)?,
            _ => return None,
        };

        Self::parse_btp_b_from_common_header(common)
    }

    /// Strips SignedData security wrapper from a secured GNW packet (Basic Header NH=2) and returns a sub-slice starting at the GNW Common Header.
    ///
    /// Wire layout after the 4-byte Basic Header (IEEE 1609.2-2016/2022 OER):
    /// ```text
    /// [03]      protocolVersion = 3
    /// [81]      Ieee1609Dot2Content: signedData
    /// [hh]      hashId (00=sha256, 01=sha384)
    /// [pp]      SignedDataPayload presence bitmap (typically 0x40 or 0x80)
    /// [03]      inner Ieee1609Dot2Data protocolVersion = 3
    /// [80]      inner Ieee1609Dot2Content: unsecuredData
    /// [ll …]   OER-encoded Opaque length  (1-3 bytes)
    /// [payload] GNW Common Header | Extended Header | BTP-B | Application UPER
    /// ```
    //
    fn strip_etsi103097(gnw_data: &[u8]) -> Option<&[u8]> {
        if gnw_data.len() < 16 { return None; }
        if gnw_data[4] != 0x03 { return None; } // protocolVersion
        if gnw_data[5] != 0x81 { return None; } // signedData

        // search for inner Ieee1609Dot2Data {protocolVersion=0x03, unsecuredData=0x80}
        // within a small window after hashId (gnw_data[6]) and the presence bitmap
        for offset in 7_usize..=12 {
            if offset + 3 > gnw_data.len() { break; }
            if gnw_data[offset] == 0x03 && gnw_data[offset + 1] == 0x80 {
                if let Some((data_len, data_start)) = Self::parse_oer_length(gnw_data, offset + 2) {
                    let end = data_start.checked_add(data_len)?;
                    if data_len >= 12 && end <= gnw_data.len() {
                        return Some(&gnw_data[data_start..end]);
                    }
                }
            }
        }

        None
    }

    /// Parses BTP-B from a slice that starts at the GNW Common Header
    /// (i.e the Basic Header has already been consumed or skipped).
    fn parse_btp_b_from_common_header(common: &[u8]) -> Option<(BtpBInfo, &[u8])> {
        if common.len() < 12 { return None; }

        let nh = common[0] >> 4;
        if nh != 2 { return None; } // not BTP-B

        let ht  = common[1] >> 4;
        let hst = common[1] & 0x0F;

        let ext_len: usize = match ht {
            1 => 24, // Beacon: SO PV (24)
            2 => 36, // GeoUnicast: SN(2) + Res(2) + SO PV(24) + DO PV(8)
            3 => 44, // GeoAnycast: SN(2) + Res(2) + SO PV(24) + Geo Area(16)
            4 => 44, // GeoBroadcast: SN(2) + Res(2) + SO PV(24) + Geo Area(16)
            5 => 28, // SHB: SO PV(24) + MDD(4) = 28 | TSB: SN(2)+Res(2)+SO PV(24) = 28
            6 => 28, // LS
            _ => return None,
        };

        // common header: 8 bytes; BTP-B starts after extended header
        let btp_off = 8 + ext_len;
        let its_off = btp_off + 4; // skip btp-b header (4 bytes)

        if common.len() < its_off { return None; }

        let port      = u16::from_be_bytes(common[btp_off..btp_off+2].try_into().unwrap());
        let port_info = u16::from_be_bytes(common[btp_off+2..btp_off+4].try_into().unwrap());

        let its_bytes = &common[its_off..];

        Some((
            BtpBInfo { destination_port: port, destination_port_info: port_info },
            its_bytes,
        ))
    }

    /// Decodes an OER/COER variable-length field starting at `pos`.
    /// Returns `(length, offset_after_length)`
    fn parse_oer_length(data: &[u8], pos: usize) -> Option<(usize, usize)> {
        if pos >= data.len() { return None; }
        let first = data[pos];
        if first <= 0x7F {
            Some((first as usize, pos + 1))
        } else if first == 0x81 {
            if pos + 1 >= data.len() { return None; }
            Some((data[pos + 1] as usize, pos + 2))
        } else if first == 0x82 {
            if pos + 2 >= data.len() { return None; }
            let len = ((data[pos + 1] as usize) << 8) | data[pos + 2] as usize;
            Some((len, pos + 3))
        } else {
            None
        }
    }

    /// Extracts position data from the GNW Source Long Position Vector
    ///
    /// Supports NH=1 (unsecured) and NH=2 (ETSI TS 103 097 signed) packets.
    /// The LPV position is derived from the Header Type/Sub-type via [`lpv_ext_offset`].
    pub fn parse_gnw(data: &[u8]) -> Option<GnwInfo> {
        let common: &[u8] = match data.first()? & 0x0F {
            1 => data.get(4..)?,
            2 => Self::strip_etsi103097(data)?,
            _ => return None,
        };

        let ht  = common.get(1)? >> 4;
        let hst = common.get(1)? & 0x0F;
        let lpv_base = GNW_COMMON_HDR_LEN + lpv_ext_offset(ht, hst)?;
        let tst_off = lpv_base + LPV_TST_OFFSET;
        let lat_off = lpv_base + LPV_LAT_OFFSET;
        let lon_off = lat_off + 4;
        let spd_off = lon_off + 4;
        let hdg_off = spd_off + 2;

        let gnw_tst = common.get(tst_off..tst_off+4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes);
        let lat     = i32::from_be_bytes(common.get(lat_off..lat_off+4)?.try_into().ok()?);
        let lon     = i32::from_be_bytes(common.get(lon_off..lon_off+4)?.try_into().ok()?);
        let spd_raw = u16::from_be_bytes(common.get(spd_off..spd_off+2)?.try_into().ok()?);
        let heading = u16::from_be_bytes(common.get(hdg_off..hdg_off+2)?.try_into().ok()?);

        Some(GnwInfo {
            latitude:  lat,
            longitude: lon,
            speed:     spd_raw & 0x7FFF,
            heading,
            gnw_timestamp_ms: gnw_tst,
        })
    }

    /// Parses one captured frame regardless of whether it carries a Radiotap
    /// header (DLT 127) or starts directly with an 802.11 MAC header (DLT 105).
    pub fn parse_live_packet(timestamp_ns: u64, data: &[u8]) -> Option<ParsedPacket> {
        if let Ok(rt) = Radiotap::from_bytes(data) {
            let rt_len = rt.header.length as usize;
            if data.len() < rt_len { return None; }
            let rssi      = rt.antenna_signal.map(|s| s.value as f64).unwrap_or(-100.0);
            let mactime   = rt.tsft.map(|t| t.value);
            Self::parse_ieee80211(&data[rt_len..], timestamp_ns, data.len(), rssi, mactime)
        } else {
            // no Radiotap header, treat as raw
            Self::parse_ieee80211(data, timestamp_ns, data.len(), -100.0, None)
        }
    }

    /// Parses the 802.11 MAC layer and everything above it
    fn parse_ieee80211(
        wlan_data: &[u8],
        timestamp_ns: u64,
        frame_len: usize,
        rssi: f64,
        mactime_us: Option<u64>,
    ) -> Option<ParsedPacket> {
        let (mac, wlan_len) = Self::parse_wlan_mac(wlan_data)?;
        if wlan_data.len() < wlan_len { return None; }

        // Sequence Control is at bytes 22-23 (LE); bits 4-15 are the sequence number
        // parse_wlan_mac guarantees len >= 24, so this read is always safe
        let frame_seq = Some(u16::from_le_bytes([wlan_data[22], wlan_data[23]]) >> 4);

        let llc = &wlan_data[wlan_len..];
        let mut gnw_info   = None;
        let mut btp_b_info = None;
        let mut payload    = None;

        if let Some(llc_len) = Self::parse_llc(llc) {
            if llc.len() >= llc_len {
                let gnw_data = &llc[llc_len..];
                gnw_info = Self::parse_gnw(gnw_data);
                if let Some((btp, its_bytes)) = Self::parse_btp_b(gnw_data) {
                    payload = Some(crate::parser::decoder::MessageRouter::decode_payload(
                        btp.destination_port,
                        its_bytes,
                    ));
                    btp_b_info = Some(btp);
                }
            }
        }

        Some(ParsedPacket {
            timestamp_ms: (timestamp_ns / 1_000_000) as i64,
            mac,
            rssi,
            data_len: frame_len,
            gnw_info,
            btp_b_info,
            payload,
            mactime_us,
            frame_seq,
        })
    }

    /// Parses a PCAPNG file from an in-memory byte slice.
    /// Used when the file was downloaded from the node rather than opened locally or for re-injection.
    pub fn parse_bytes_raw(data: &[u8]) -> std::io::Result<Vec<(ParsedPacket, Vec<u8>)>> {
        let cursor = std::io::Cursor::new(data);
        let mut reader = PcapNgReader::new(cursor)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let mut packets = Vec::new();
        while let Some(block) = reader.next_block() {
            let Ok(block) = block else { break };
            match block {
                Block::EnhancedPacket(epb) => {
                    let ts_ns = epb.timestamp.as_nanos() as u64;
                    let raw = epb.data.to_vec();
                    if let Some(parsed) = Self::parse_live_packet(ts_ns, &raw) {
                        packets.push((parsed, raw));
                    }
                }
                Block::SimplePacket(spb) => {
                    let raw = spb.data.to_vec();
                    if let Some(parsed) = Self::parse_live_packet(0, &raw) {
                        packets.push((parsed, raw));
                    }
                }
                _ => {}
            }
        }
        Ok(packets)
    }

    // TODO: maybe merge with parse_bytes_raw at some point
    pub fn parse_bytes(data: &[u8]) -> std::io::Result<Vec<ParsedPacket>> {
        let cursor = std::io::Cursor::new(data);
        let mut reader = PcapNgReader::new(cursor)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let mut packets = Vec::new();
        while let Some(block) = reader.next_block() {
            if let Ok(block) = block {
                match block {
                    Block::EnhancedPacket(epb) => {
                        let ts_ns = epb.timestamp.as_nanos() as u64;
                        if let Some(parsed) = Self::parse_live_packet(ts_ns, &epb.data) {
                            packets.push(parsed);
                        }
                    }
                    Block::SimplePacket(spb) => {
                        if let Some(parsed) = Self::parse_live_packet(0, &spb.data) {
                            packets.push(parsed);
                        }
                    }
                    _ => {}
                }
            } else {
                break;
            }
        }
        Ok(packets)
    }

    /// parse a pcapng flie from disk
    pub fn parse_file(path: impl AsRef<Path>) -> std::io::Result<Vec<ParsedPacket>> {
        let file = File::open(path)?;
        let mut reader = PcapNgReader::new(file).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        
        let mut packets = Vec::new();
        while let Some(block) = reader.next_block() {
            if let Ok(block) = block {
                match block {
                    Block::EnhancedPacket(epb) => {
                        let ts_ns = epb.timestamp.as_nanos() as u64;
                        if let Some(parsed) = Self::parse_live_packet(ts_ns, &epb.data) {
                            packets.push(parsed);
                        }
                    }
                    Block::SimplePacket(spb) => {
                        let ts_ns = 0;
                        if let Some(parsed) = Self::parse_live_packet(ts_ns, &spb.data) {
                            packets.push(parsed);
                        }
                    }
                    _ => {}
                }
            } else {
                break;
            }
        }
        
        Ok(packets)
    }
}

