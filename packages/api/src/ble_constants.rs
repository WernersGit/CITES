/// Central constants for Bluetooth Low Energy (BLE) communication.
/// Both the backend (Raspberry Pi) and the client (App) must use these
/// exact UUIDs to discover and communicate with eachother exclusively.

// Base UUID: c17e5000-babe-4b1e-8256-000000000000 (CITES-000...)
pub const CITES_SERVICE_UUID: &str = "c17e5000-babe-4b1e-8256-000000000000";

// Characteristic for streeaming System Metrics (CPU, RAM, Temp) - READ / NOTIFY
pub const METRICS_CHARACTERISTIC_UUID: &str = "c17e5001-babe-4b1e-8256-000000000000";

// Characteristic for receiving PCAP-Data / Reports - READ / NOTIFY
pub const PCAP_CHARACTERISTIC_UUID: &str = "c17e5002-babe-4b1e-8256-000000000000";

// Characteristic for sending commands (e.g. Start/Stop Capture) - WRITE
pub const COMMAND_CHARACTERISTIC_UUID: &str = "c17e5003-babe-4b1e-8256-000000000000";

// Characteristic for reading archive filenames + valid packet counts - READ
pub const ARCHIVE_LIST_CHARACTERISTIC_UUID: &str = "c17e5004-babe-4b1e-8256-000000000000";

// Characteristic for reading injection engine status - READ
pub const INJECTION_STATUS_CHARACTERISTIC_UUID: &str = "c17e5005-babe-4b1e-8256-000000000000";

// Characteristic for reading/pushing node runtime config (log level, port) - READ
pub const NODE_CONFIG_CHARACTERISTIC_UUID: &str = "c17e5006-babe-4b1e-8256-000000000000";

// Characteristic for the application-level session handshake - READ / WRITE.
// The client writes a HandshakeFrame with its max chunk size; subsequent READs
// return teh server-reconciled frame that both sides MUST use for the session.
// See `core_logic::ble_handshake` for the wire format.
pub const HANDSHAKE_CHARACTERISTIC_UUID: &str = "c17e5007-babe-4b1e-8256-000000000000";

/// First byte of any extended (non-legacy) BLE command written to COMMAND_CHARACTERISTIC.
/// Legacy `RepeatModeConfig` commands start with a flags byte whose upper 6 bits are
/// always zero, so 0xFF safely distinguishes new commands.
pub const BLE_CMD_MAGIC: u8 = 0xFF;

pub const BLE_CMD_START_INJECTION:  u8 = 0x10;
pub const BLE_CMD_STOP_INJECTION:   u8 = 0x11;
pub const BLE_CMD_PAUSE_INJECTION:  u8 = 0x12;
/// 3-byte frame: [MAGIC, SET_INJ_FILTER, 0|1]
pub const BLE_CMD_SET_INJ_FILTER:   u8 = 0x13;
pub const BLE_CMD_SET_NODE_CONFIG:  u8 = 0x20;

/// Prefix for the local BLE name advertised by the Raspberry Pi.
pub const CITES_MAC_NAME_PREFIX: &str = "CITES-Node-";
