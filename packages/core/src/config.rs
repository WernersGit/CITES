use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ItsStandard {
    Release1,
    Release2,
}

/// Application-level ITS protocols that can be selected for replay filtering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ReplayProtocol {
    Cam,
    Denm,
    Ivim,
    Mapem,
    Spatem,
    Cpm,
    Srem,
    Ssem,
}

impl ReplayProtocol {
    pub const ALL: &'static [ReplayProtocol] = &[
        Self::Cam,
        Self::Denm,
        Self::Ivim,
        Self::Mapem,
        Self::Spatem,
        Self::Cpm,
        Self::Srem,
        Self::Ssem,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cam => "CAM",
            Self::Denm => "DENM",
            Self::Ivim => "IVIM",
            Self::Mapem => "MAPEM",
            Self::Spatem => "SPATEM",
            Self::Cpm => "CPM",
            Self::Srem => "SREM",
            Self::Ssem => "SSEM",
        }
    }
}

/// Configuration for the replay mode sent from the client to the node.
///
/// When `enabled` is true, the node will immediately replay incoming messages
/// that match both filters. An absent `vehicle_id_filter` means all vehicles;
/// an empty `protocol_filter` means all protocols. Both filters are ANDed.
/// `delay_ms` inserts a pause after each replayed packet to throttle througput.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepeatModeConfig {
    pub enabled: bool,
    /// `None` -> replay messages from all vehicles
    pub vehicle_id_filter: Option<u32>,
    /// Empty -> replay all protocols. Non-empty -> only the listed protocols
    pub protocol_filter: Vec<ReplayProtocol>,
    /// Milliseconds to sleep after each replayed packet (0 = no delay) 
    pub delay_ms: u64,
    /// Transmit power in dBm. `None` -> use the interface default
    /// Hardware maximum of the installed technology is +27 dBm
    #[serde(default)]
    pub tx_power_dbm: Option<u8>,
}

impl Default for RepeatModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vehicle_id_filter: None,
            protocol_filter: vec![],
            delay_ms: 0,
            tx_power_dbm: None,
        }
    }
}

impl RepeatModeConfig {
    /// Serialises the config into a compact 11-byte binary frame for BLE writes.
    ///
    /// Layout:
    /// ```text
    /// [0]   flags bit 0 = enabled, bit 1 = has vehicle_id_filter
    /// [1-4] vehicle_id  u32 big-endian (0 when no filter)
    /// [5]   protocol bitmask  bit N -> ReplayProtocol (see below)
    /// [6-9] delay_ms u32 big-endian
    /// [10]  tx_power_dbm 0xFF = None -> use default
    /// ```
    /// Protocol bitmask: Cam=0x01, Denm=0x02, Ivim=0x04, Mapem=0x08,
    ///                   Spatem=0x10, Cpm=0x20, Srem=0x40, Ssem=0x80
    pub fn to_ble_binary(&self) -> [u8; 11] {
        let mut flags = 0u8;
        if self.enabled {
            flags |= 0x01;
        }
        if self.vehicle_id_filter.is_some() {
            flags |= 0x02;
        }

        let vid = self.vehicle_id_filter.unwrap_or(0).to_be_bytes();

        let mut proto_mask = 0u8;
        for proto in &self.protocol_filter {
            proto_mask |= match proto {
                ReplayProtocol::Cam    => 0x01,
                ReplayProtocol::Denm   => 0x02,
                ReplayProtocol::Ivim   => 0x04,
                ReplayProtocol::Mapem  => 0x08,
                ReplayProtocol::Spatem => 0x10,
                ReplayProtocol::Cpm    => 0x20,
                ReplayProtocol::Srem   => 0x40,
                ReplayProtocol::Ssem   => 0x80,
            };
        }

        let delay = (self.delay_ms.min(u32::MAX as u64) as u32).to_be_bytes();
        let tx = self.tx_power_dbm.unwrap_or(0xFF);

        [
            flags,
            vid[0], vid[1], vid[2], vid[3],
            proto_mask,
            delay[0], delay[1], delay[2], delay[3],
            tx,
        ]
    }

    /// Deserialises from the compact binary frame. Returns `None` if `bytes`
    /// is shorter than 10 bytes. Byte [10] (tx_power_dbm) is optional:
    /// absent or 0xFF -> `None` -> use interface default
    pub fn from_ble_binary(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 10 {
            return None; // bad input
        }

        let flags = bytes[0];
        let enabled = flags & 0x01 != 0;
        let has_vid = flags & 0x02 != 0;

        let vid_raw = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
        let vid_filter = if has_vid { Some(vid_raw) } else { None };

        let proto_mask = bytes[5];
        let candidates: &[(u8, ReplayProtocol)] = &[
            (0x01, ReplayProtocol::Cam),
            (0x02, ReplayProtocol::Denm),
            (0x04, ReplayProtocol::Ivim),
            (0x08, ReplayProtocol::Mapem),
            (0x10, ReplayProtocol::Spatem),
            (0x20, ReplayProtocol::Cpm),
            (0x40, ReplayProtocol::Srem),
            (0x80, ReplayProtocol::Ssem),
        ];
        let proto_f = candidates
            .iter()
            .filter(|(bit, _)| proto_mask & bit != 0)
            .map(|(_, proto)| *proto)
            .collect();

        let delay_ms = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as u64;

        let tx_power_dbm = bytes.get(10).and_then(|&b| {
            if b == 0xFF { None } else { Some(b) }
        });

        Some(Self { enabled, vehicle_id_filter: vid_filter, protocol_filter: proto_f, delay_ms, tx_power_dbm })
    }
}

/// Configuration for the tracking-warning feature
///
/// Two independent thresholds can trigger a warning; the first one reached fires.
/// After a warning the window resets so the vehicle can trigger again.
///
/// Intended use: time threshold for urban scenarios (short distances), distance threshold for extra-urban scenarios.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackingWarningConfig {
    pub enabled: bool,
    /// Minutes a vehicle must remain continuously visible before the duration warning fires.
    pub min_visible_minutes: u32,
    /// Seconds a vehicle may be absent between sightings without resetting the time window.
    pub gap_tolerance_secs: u32,
    /// Kilometres a vehicle must travel continuously before the distance warning fires.
    pub min_visible_km: f64,
    /// Kilometres of positional gap still considered continuous travel.
    pub gap_tolerance_km: f64,
}

impl Default for TrackingWarningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_visible_minutes: 5,
            gap_tolerance_secs: 30,
            min_visible_km: 5.0,
            gap_tolerance_km: 0.5,
        }
    }
}

// injection types

/// How many times to repeat a full injection pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum RepeatMode {
    /// Inject the filtered packet set exactly once
    #[default]
    Once,
    /// Inject the filtered packet set N times
    Count(u32),
    /// Inject indefinitely until stopped
    Infinite,
}

/// Timing and repetition schedule for packet injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InjectionSchedule {
    pub repeat: RepeatMode,
    /// Milliseconds to sleep between consecutive packets (0 = no delay)
    pub packet_delay_ms: u64,
    /// Milliseconds to sleep between repetition loops (ignored for `Once`)
    pub loop_delay_ms: u64,
    /// When true, replays at original inter-packet intervals; overrides `packet_delay_ms`
    pub preserve_timing: bool,
    /// Adds 0..=jitter_ms ms of deterministic variation per packet
    pub jitter_ms: u64,
}

impl Default for InjectionSchedule {
    fn default() -> Self {
        Self {
            repeat: RepeatMode::Once,
            packet_delay_ms: 0,
            loop_delay_ms: 1000,
            preserve_timing: true,
            jitter_ms: 0,
        }
    }
}

/// Packet-level filter applied before injection.
///
/// All active fields are ANDed; an absent or empty field means no restriction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct InjectionFilter {
    /// Empty -> all protocols
    pub protocols: Vec<ReplayProtocol>,
    /// `None` -> all vehicles 
    pub vehicle_id: Option<u32>,
    /// Millisecond offset from first packet (inclusive lower bound).  `None` -> no limit
    pub time_range_start_ms: Option<u64>,
    /// Millisecond offset from first packet (inclusive upper bound).  `None` -> no limit
    pub time_range_end_ms: Option<u64>,
}

/// Full configuration for a single injection run sent from the client to the node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InjectionConfig {
    /// Filename (no path separators) of the PCAPNG archive on the node.
    pub archive_filename: String,
    pub filter: InjectionFilter,
    pub schedule: InjectionSchedule,
    /// When true, counts packets without actually sending them.
    pub dry_run: bool,
    /// Transmit power in dBm -> `None` -> use the interface default.
    /// Hardware maximum of the installed technology is +27 dBm.
    #[serde(default)]
    pub tx_power_dbm: Option<u8>,
    /// When true (default), the capture thread drops frames whose FCS matches
    /// the injected loopback. Disable for debug/development to observe the
    /// raw loopback traffic at the client.
    #[serde(default = "default_filter_inj")]
    pub filter_inj: bool,
}

fn default_filter_inj() -> bool { true }

/// Reported state of the injection engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum InjectionEngineState {
    #[default]
    Idle,
    Running,
    Paused,
    Completed,
    Error(String),
}

impl InjectionEngineState {
    pub fn label(&self) -> &str {
        match self {
            Self::Idle      => "Idle",
            Self::Running   => "Running",
            Self::Paused    => "Paused",
            Self::Completed => "Completed",
            Self::Error(_)  => "Error",
        }
    }
}

/// Status snapshot reported by the injection engine via `GET /injection/status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct InjectionStatus {
    pub state: InjectionEngineState,
    pub packets_sent: u64,
    pub packets_total: u64,
    pub current_iteration: u32,
    pub elapsed_ms: u64,
    /// Current capture-filter toggle state on the node. Used by the client UI
    /// to reflect mid-run filter changes pushed via `POST /injection/filter`.
    #[serde(default)]
    pub filter_inj: bool,
}

impl InjectionConfig {
    /// Compact bianry encoding for BLE transport
    ///
    /// Layout (fixed 29 bytes + filename):
    /// ```text
    /// [0]         BLE_CMD_MAGIC (0xFF)
    /// [1]         BLE_CMD_START_INJECTION (0x10)
    /// [2]         flags: bit0=dry_run, bit1=pres_timing, bit2=infinite_repeat, bit3=disable_filter (0 = filter on, 1 = filter off)
    /// [3]         tx_power_dbm (0xFF = None)
    /// [4]         protocol_bitmask (same encoding as RepeatModeConfig)
    /// [5..8]      vehicle_id u32 BE (0 = no filter)
    /// [9..12]     packet_delay_ms u32 BE
    /// [13..16]    loop_delay_ms u32 BE
    /// [17..20]    jitter_ms u32 BE
    /// [21..24]    time_range_start_ms u32 BE (0 = no limit)
    /// [25..28]    time_range_end_ms u32 BE (0xFFFF_FFFF = no limit)
    /// [29..]      archive_filename bytes (no terminator; consume to end)
    /// ```
    pub fn to_ble_binary(&self) -> Vec<u8> {
        const MAGIC: u8 = 0xFF;
        const CMD:   u8 = 0x10;

        let mut flags = 0u8;
        if self.dry_run { flags |= 0x01; }
        if self.schedule.preserve_timing { flags |= 0x02; }
        if matches!(self.schedule.repeat, RepeatMode::Infinite) { flags |= 0x04; }
        // bit3 is set when the filter is DISABLED, so old clients (bit3=0)
        // get the default-on behaviour without any wire-format upgrade
        if !self.filter_inj { flags |= 0x08; }

        let proto_mask = self.filter.protocols.iter().fold(0u8, |acc, p| acc | match p {
            ReplayProtocol::Cam    => 0x01,
            ReplayProtocol::Denm   => 0x02,
            ReplayProtocol::Ivim   => 0x04,
            ReplayProtocol::Mapem  => 0x08,
            ReplayProtocol::Spatem => 0x10,
            ReplayProtocol::Cpm    => 0x20,
            ReplayProtocol::Srem   => 0x40,
            ReplayProtocol::Ssem   => 0x80,
        });

        let vid   = self.filter.vehicle_id.unwrap_or(0).to_be_bytes();
        let delay = (self.schedule.packet_delay_ms.min(u32::MAX as u64) as u32).to_be_bytes();
        let ldly  = (self.schedule.loop_delay_ms.min(u32::MAX as u64) as u32).to_be_bytes();
        let jit   = (self.schedule.jitter_ms.min(u32::MAX as u64) as u32).to_be_bytes();
        let trs   = self.filter.time_range_start_ms.map(|v| v.min(u32::MAX as u64) as u32).unwrap_or(0).to_be_bytes();
        let tre   = self.filter.time_range_end_ms.map(|v| v.min(u32::MAX as u64) as u32).unwrap_or(u32::MAX).to_be_bytes();
        let tx    = self.tx_power_dbm.unwrap_or(0xFF);

        let mut out = vec![
            MAGIC, CMD,
            flags, tx, proto_mask,
            vid[0], vid[1], vid[2], vid[3],
            delay[0], delay[1], delay[2], delay[3],
            ldly[0], ldly[1], ldly[2], ldly[3],
            jit[0], jit[1], jit[2], jit[3],
            trs[0], trs[1], trs[2], trs[3],
            tre[0], tre[1], tre[2], tre[3],
        ];
        out.extend_from_slice(self.archive_filename.as_bytes());
        out
    }

    /// Deserialises from the compact BLE binary produced by [`to_ble_binary`].
    pub fn from_ble_binary(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 30 { return None; }
        if bytes[0] != 0xFF || bytes[1] != 0x10 { return None; }

        let flags    = bytes[2];
        let dry_run  = flags & 0x01 != 0;
        let pres     = flags & 0x02 != 0;
        let infinite = flags & 0x04 != 0;
        // bit3 set means "filter off"; default (bit3=0) keeps the filter on
        let filter_inj = flags & 0x08 == 0;

        let tx_power_dbm = match bytes[3] { 0xFF => None, v => Some(v) };
        let proto_mask   = bytes[4];

        let proto_map: &[(u8, ReplayProtocol)] = &[
            (0x01, ReplayProtocol::Cam),    (0x02, ReplayProtocol::Denm),
            (0x04, ReplayProtocol::Ivim),   (0x08, ReplayProtocol::Mapem),
            (0x10, ReplayProtocol::Spatem), (0x20, ReplayProtocol::Cpm),
            (0x40, ReplayProtocol::Srem),   (0x80, ReplayProtocol::Ssem),
        ];
        let protocols = proto_map.iter()
            .filter(|(bit, _)| proto_mask & bit != 0)
            .map(|(_, p)| *p)
            .collect();

        let vid   = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
        let delay = u32::from_be_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]) as u64;
        let ldly  = u32::from_be_bytes([bytes[13], bytes[14], bytes[15], bytes[16]]) as u64;
        let jit   = u32::from_be_bytes([bytes[17], bytes[18], bytes[19], bytes[20]]) as u64;
        let trs   = u32::from_be_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
        let tre   = u32::from_be_bytes([bytes[25], bytes[26], bytes[27], bytes[28]]);

        let fname = String::from_utf8(bytes[29..].to_vec()).ok()?;
        if fname.is_empty() { return None; }

        Some(Self {
            archive_filename: fname,
            filter: InjectionFilter {
                protocols,
                vehicle_id: if vid == 0 { None } else { Some(vid) },
                time_range_start_ms: if trs == 0 { None } else { Some(trs as u64) },
                time_range_end_ms: if tre == u32::MAX { None } else { Some(tre as u64) },
            },
            schedule: InjectionSchedule {
                repeat: if infinite { RepeatMode::Infinite } else { RepeatMode::Once },
                packet_delay_ms: delay,
                loop_delay_ms: ldly,
                preserve_timing: pres,
                jitter_ms: jit,
            },
            dry_run,
            tx_power_dbm,
            filter_inj,
        })
    }
}

impl InjectionStatus {
    /// 12-byte binary status for BLE INJECTION_STATUS_CHARACTERISTIC reads.
    ///
    /// Layout: `[state][sent u32 BE][total u32 BE][elapsed_s u16 BE][flags]` where flags bit0 = `filter_inj`.
    pub fn to_ble_binary(&self) -> [u8; 12] {
        let state = match &self.state {
            InjectionEngineState::Idle      => 0u8,
            InjectionEngineState::Running   => 1,
            InjectionEngineState::Paused    => 2,
            InjectionEngineState::Completed => 3,
            InjectionEngineState::Error(_)  => 4,
        };
        let sent  = (self.packets_sent.min(u32::MAX as u64) as u32).to_be_bytes();
        let total = (self.packets_total.min(u32::MAX as u64) as u32).to_be_bytes();
        let elap  = ((self.elapsed_ms / 1000).min(u16::MAX as u64) as u16).to_be_bytes();
        let flags = if self.filter_inj { 0x01 } else { 0 };
        [
            state,
            sent[0], sent[1], sent[2], sent[3],
            total[0], total[1], total[2], total[3],
            elap[0], elap[1],
            flags,
        ]
    }

    /// Deserialises from the 12-byte BLE binary produced by [`to_ble_binary`].
    pub fn from_ble_binary(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 11 { return None; }
        let state = match bytes[0] {
            0 => InjectionEngineState::Idle,
            1 => InjectionEngineState::Running,
            2 => InjectionEngineState::Paused,
            3 => InjectionEngineState::Completed,
            4 => InjectionEngineState::Error("BLE error".into()),
            _ => return None,
        };
        let sent  = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as u64;
        let total = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as u64;
        let elap  = u16::from_be_bytes([bytes[9], bytes[10]]) as u64 * 1000;
        // older nodes do not write byte 11; default to filter_inj = true for safety
        let filter_inj = bytes.get(11).map(|b| b & 0x01 != 0).unwrap_or(true);
        Some(Self {
            state,
            packets_sent: sent,
            packets_total: total,
            elapsed_ms: elap,
            current_iteration: 0,
            filter_inj,
        })
    }
}


// NodeConfig

/// Verbosity level for the node daemon's logger
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const ALL: &'static [LogLevel] = &[
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info  => "INFO",
            Self::Warn  => "WARN",
            Self::Error => "ERROR",
        }
    }

    /// Filter string accepted by `tracing_subscriber::EnvFilter`
    pub fn as_filter_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info  => "info",
            Self::Warn  => "warn",
            Self::Error => "error",
        }
    }

    fn to_u8(self) -> u8 {
        match self { Self::Debug => 0, Self::Info => 1, Self::Warn => 2, Self::Error => 3 }
    }

    fn from_u8(b: u8) -> Option<Self> {
        match b { 0 => Some(Self::Debug), 1 => Some(Self::Info), 2 => Some(Self::Warn), 3 => Some(Self::Error), _ => None }
    }

    /// parses the filter strings produced by [`as_filter_str`]
    pub fn from_filter_str(s: &str) -> Option<Self> {
        match s { "debug" => Some(Self::Debug), "info" => Some(Self::Info), "warn" => Some(Self::Warn), "error" => Some(Self::Error), _ => None }
    }
}

/// Runtime-adjustable node configuration pushed from the client UI.
///
/// Log level is applied immediately; API port requires a node restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeConfig {
    pub log_level: LogLevel,
    /// HTTP API port (default 8080): change takes effect after node restart
    pub api_port: u16,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self { log_level: LogLevel::default(), api_port: 8080 }
    }
}

impl NodeConfig {
    /// 5-byte BLE frame: `[0xFF][0x20][log_level][port_hi][port_lo]` 
    pub fn to_ble_binary(&self) -> [u8; 5] {
        let p = self.api_port.to_be_bytes();
        [0xFF, 0x20, self.log_level.to_u8(), p[0], p[1]]
    }

    pub fn from_ble_binary(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 5 || bytes[0] != 0xFF || bytes[1] != 0x20 { return None; }
        Some(Self {
            log_level: LogLevel::from_u8(bytes[2])?,
            api_port: u16::from_be_bytes([bytes[3], bytes[4]]),
        })
    }

    /// 3-byte payload for the BLE `NODE_CONFIG_CHARACTERISTIC` READ:
    /// `[log_level][port_hi][port_lo]` (no magic prefix needed for read).
    pub fn to_ble_status(&self) -> [u8; 3] {
        let p = self.api_port.to_be_bytes();
        [self.log_level.to_u8(), p[0], p[1]]
    }

    pub fn from_ble_status(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 3 { return None; }
        Some(Self {
            log_level: LogLevel::from_u8(bytes[0])?,
            api_port: u16::from_be_bytes([bytes[1], bytes[2]]),
        })
    }
}

// EgoMacConfig

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgoMacConfig {
    pub min_seconds_before_eval: f64,
    pub min_messages_before_eval: usize,
    pub rolling_window: usize,
    pub min_periods: usize,
    pub eval_interval_ms: u64,
}

impl Default for EgoMacConfig {
    fn default() -> Self {
        Self {
            min_seconds_before_eval: 20.0,
            min_messages_before_eval: 30,
            rolling_window: 20,
            min_periods: 5,
            eval_interval_ms: 1000,
        }
    }
}
