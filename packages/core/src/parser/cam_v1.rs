use std::os::raw::{c_long, c_void};

extern "C" { fn free(ptr: *mut c_void); }
use crate::asn1::cam::v1::{
    CAM_t,
    asn_DEF_CAM,
    HighFrequencyContainer_PR_HighFrequencyContainer_PR_basicVehicleContainerHighFrequency,
    LowFrequencyContainer_PR_LowFrequencyContainer_PR_basicVehicleContainerLowFrequency,
    uper_decode_complete,
    asn_dec_rval_code_e_RC_OK,
    asn_INTEGER2long,
};
use super::types::{AccelControl, DecodedCam, ExteriorLights};

/// YawRateConfidnce (0–8) -> deg/s; 7/8 map to None
fn yaw_rate_conf_deg_s(val: u8) -> Option<f64> {
    const TABLE: [f64; 7] = [0.01, 0.05, 0.10, 1.00, 5.00, 10.0, 100.0];
    TABLE.get(val as usize).copied()
}

pub fn decode(data: &[u8]) -> Option<DecodedCam> {
    unsafe {
        let def = std::ptr::addr_of!(asn_DEF_CAM);
        let mut cam_ptr: *mut CAM_t = std::ptr::null_mut();

        let rval = uper_decode_complete(
            std::ptr::null_mut(),
            def,
            &mut cam_ptr as *mut *mut CAM_t as *mut *mut c_void,
            data.as_ptr() as *const c_void,
            data.len(),
        );

        let result = if rval.code == asn_dec_rval_code_e_RC_OK && !cam_ptr.is_null() {
            Some(extract(&*cam_ptr))
        } else {
            None
        };

        if !cam_ptr.is_null() {
            free(cam_ptr as *mut c_void);
        }

        result
    }
}

/// Reads a long from an asn1c ENUMERATED_t / INTEGER_t field
/// Returns 0 on failure (which is usually the "forward" / default sentinel)
unsafe fn enum_to_long(val: *const crate::asn1::cam::v1::INTEGER_t) -> c_long {
    let mut out: c_long = 0;
    asn_INTEGER2long(val, &mut out);
    out
}

unsafe fn extract(cam: &CAM_t) -> DecodedCam {
    let mut result = DecodedCam {
        protocol_version: 0,
        station_id: 0,
        gen_delta_time_ms: 0,
        station_type: 0,
        latitude: None,
        longitude: None,
        altitude_m: None,
        pos_confidence_m: None,
        speed_kmh: None,
        speed_confidence_ms: None,
        heading_deg: None,
        heading_confidence_deg: None,
        drive_direction: None,
        vehicle_length_m: None,
        vehicle_width_m: None,
        longitudinal_accel: None,
        curvature: None,
        yaw_rate: None,
        yaw_rate_confidence_deg_s: None,
        accel_control: None,
        vehicle_role: None,
        exterior_lights: None,
    };

    if let Some(header) = cam.header.as_ref() {
        result.protocol_version = header.protocolVersion as u8;
        result.station_id = header.stationID as u32;
    }

    let Some(coop) = cam.cam.as_ref() else { return result; };
    result.gen_delta_time_ms = coop.generationDeltaTime as u32;

    let Some(params) = coop.camParameters.as_ref() else { return result; };

    // BasicContainer
    if let Some(basic) = params.basicContainer.as_ref() {
        result.station_type = basic.stationType as u32;

        if let Some(ref_pos) = basic.referencePosition.as_ref() {
            let lat_raw = ref_pos.latitude as i64;
            if lat_raw.abs() <= 900_000_000 {
                result.latitude = Some(lat_raw as f64 * 1e-7);
            }
            let lon_raw = ref_pos.longitude as i64;
            if lon_raw.abs() <= 1_800_000_000 {
                result.longitude = Some(lon_raw as f64 * 1e-7);
            }
            if let Some(alt) = ref_pos.altitude.as_ref() {
                let alt_raw = alt.altitudeValue as i64;
                if alt_raw != 800_001 {
                    result.altitude_m = Some(alt_raw as f64 * 0.01);
                }
            }
            // PosConfidenceEllipse: semiMajorConfidence in cm; 4095 -> unavailable
            if let Some(pce) = ref_pos.positionConfidenceEllipse.as_ref() {
                let smc = pce.semiMajorConfidence as i64;
                if smc > 0 && smc < 4095 {
                    result.pos_confidence_m = Some(smc as f64 * 0.01);
                }
            }
        }
    }

    // HighFrequencyContainer
    if let Some(hfc) = params.highFrequencyContainer.as_ref() {
        if hfc.present == HighFrequencyContainer_PR_HighFrequencyContainer_PR_basicVehicleContainerHighFrequency {
            let hf_ptr = hfc.choice.basicVehicleContainerHighFrequency;
            if let Some(hf) = hf_ptr.as_ref() {
                // Speed: 0.01 m/s -> km/h (* 0.036); 16383 = unavailable
                // SpeedConfidence: 0.01 m/s per unit; 127 = unavailable
                if let Some(spd) = hf.speed.as_ref() {
                    let sv = spd.speedValue as i64;
                    if sv < 16383 {
                        result.speed_kmh = Some(sv as f64 * 0.036);
                    }
                    let sc = spd.speedConfidence as i64;
                    if sc > 0 && sc < 127 {
                        result.speed_confidence_ms = Some(sc as f64 * 0.01);
                    }
                }

                // Heading: 0.1 degrees; 3601 = unavailable
                // HeadingConfidence: 0.1 degrees per unit -> 127 = unavailable
                if let Some(hdg) = hf.heading.as_ref() {
                    let h = hdg.headingValue as i64;
                    if h < 3601 {
                        result.heading_deg = Some(h as f64 * 0.1);
                    }
                    let hc = hdg.headingConfidence as i64;
                    if hc > 0 && hc < 127 {
                        result.heading_confidence_deg = Some(hc as f64 * 0.1);
                    }
                }

                //DriveDirection is ENUMERATED_t (not a c_long); use asn_INTEGER2long
                let dd = enum_to_long(&hf.driveDirection as *const _ as *const _);
                if dd < 2 {
                    result.drive_direction = Some(dd as u8);
                }

                // VehicleLength: dm -> m; 1023 = unavailable
                if let Some(vl) = hf.vehicleLength.as_ref() {
                    let l = vl.vehicleLengthValue as i64;
                    if l < 1023 {
                        result.vehicle_length_m = Some(l as f64 * 0.1);
                    }
                }

                // VehicleWidth: dm -> m; 62 = unavailable
                let w = hf.vehicleWidth as i64;
                if w > 0 && w < 62 {
                    result.vehicle_width_m = Some(w as f64 * 0.1);
                }

                // LongitudinalAcceleration: 0.1 m/s^2 -> m/s^2; 161 = unavailable
                if let Some(la) = hf.longitudinalAcceleration.as_ref() {
                    let a = la.longitudinalAccelerationValue as i64;
                    if a != 161 {
                        result.longitudinal_accel = Some(a as f64 * 0.1);
                    }
                }

                // Curvature: 30001 = unavailable
                if let Some(curv) = hf.curvature.as_ref() {
                    let c = curv.curvatureValue as i32;
                    if c != 30001 {
                        result.curvature = Some(c);
                    }
                }

                // YawRate: 0.01 deg/s; 32767 = unavailable
                // YawRateConfidence: ENUMERATED, mapped to deg/s via lookup table
                if let Some(yr) = hf.yawRate.as_ref() {
                    let y = yr.yawRateValue as i32;
                    if y != 32767 {
                        result.yaw_rate = Some(y as f64 * 0.01);
                    }
                    let yc = enum_to_long(&yr.yawRateConfidence as *const _ as *const _);
                    result.yaw_rate_confidence_deg_s = yaw_rate_conf_deg_s(yc as u8);
                }

                // AccelerationControl
                if !hf.accelerationControl.is_null() {
                    let ac = &*hf.accelerationControl;
                    if !ac.buf.is_null() && ac.size > 0 {
                        let byte = *ac.buf;
                        result.accel_control = Some(AccelControl {
                            brake_pedal_active:        (byte >> 7) & 1 == 1,
                            gas_pedal_active:          (byte >> 6) & 1 == 1,
                            emergency_brake_engaged:   (byte >> 5) & 1 == 1,
                            collision_warning_engaged: (byte >> 4) & 1 == 1,
                            acc_engaged:               (byte >> 3) & 1 == 1,
                            cruise_control_active:     (byte >> 2) & 1 == 1,
                            speed_limiter_active:      (byte >> 1) & 1 == 1,
                        });
                    }
                }
            }
        }
    }

    // LowFrequencyContainer
    if let Some(lfc) = params.lowFrequencyContainer.as_ref() {
        if lfc.present == LowFrequencyContainer_PR_LowFrequencyContainer_PR_basicVehicleContainerLowFrequency {
            let lf_ptr = lfc.choice.basicVehicleContainerLowFrequency;
            if let Some(lf) = lf_ptr.as_ref() {
                // VehicleRole is ENUMERATED_t
                let vr = enum_to_long(&lf.vehicleRole as *const _ as *const _);
                result.vehicle_role = Some(vr as u8);

                let el = &lf.exteriorLights;
                if !el.buf.is_null() && el.size > 0 {
                    let byte = *el.buf;
                    result.exterior_lights = Some(ExteriorLights {
                        low_beam:        (byte >> 7) & 1 == 1,
                        high_beam:       (byte >> 6) & 1 == 1,
                        left_turn:       (byte >> 5) & 1 == 1,
                        right_turn:      (byte >> 4) & 1 == 1,
                        daytime_running: (byte >> 3) & 1 == 1,
                        reverse_light:   (byte >> 2) & 1 == 1,
                        fog_light:       (byte >> 1) & 1 == 1,
                        parking_light:   byte & 1 == 1,
                    });
                }
            }
        }
    }

    result
}
