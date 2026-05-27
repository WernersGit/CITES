use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedCam {
    pub protocol_version: u8,
    pub station_id: u32,
    pub gen_delta_time_ms: u32,

    // BasicContainer
    pub station_type: u32,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f64>,

    // HighFrequencyContainer
    pub speed_kmh: Option<f64>,
    pub speed_confidence_ms: Option<f64>,
    pub heading_deg: Option<f64>,
    pub heading_confidence_deg: Option<f64>,
    pub drive_direction: Option<u8>,
    pub vehicle_length_m: Option<f64>,
    pub vehicle_width_m: Option<f64>,
    pub longitudinal_accel: Option<f64>,
    pub curvature: Option<i32>,
    pub yaw_rate: Option<f64>,
    pub yaw_rate_confidence_deg_s: Option<f64>,
    pub accel_control: Option<AccelControl>,
    // BasicContainer position confidence
    pub pos_confidence_m: Option<f64>,

    // LowFrequencyContainer
    pub vehicle_role: Option<u8>,
    pub exterior_lights: Option<ExteriorLights>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccelControl {
    pub brake_pedal_active: bool,
    pub gas_pedal_active: bool,
    pub emergency_brake_engaged: bool,
    pub collision_warning_engaged: bool,
    pub acc_engaged: bool,
    pub cruise_control_active: bool,
    pub speed_limiter_active: bool,
}

impl AccelControl {
    /// Returns the 7-bit bitmask with bits packed MSB-first (bit7=brake, bit6=gas, ..., bit1=speed_limiter)
    pub fn to_byte(&self) -> u8 {
        let mut b = 0u8;
        if self.brake_pedal_active        { b |= 1 << 7; }
        if self.gas_pedal_active          { b |= 1 << 6; }
        if self.emergency_brake_engaged   { b |= 1 << 5; }
        if self.collision_warning_engaged { b |= 1 << 4; }
        if self.acc_engaged               { b |= 1 << 3; }
        if self.cruise_control_active     { b |= 1 << 2; }
        if self.speed_limiter_active      { b |= 1 << 1; }
        b
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExteriorLights {
    pub low_beam: bool,
    pub high_beam: bool,
    pub left_turn: bool,
    pub right_turn: bool,
    pub daytime_running: bool,
    pub reverse_light: bool,
    pub fog_light: bool,
    pub parking_light: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedDenm {
    pub station_id: u32,
    pub protocol_version: u8,
}
