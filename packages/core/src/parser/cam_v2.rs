use std::os::raw::{c_long, c_void};
use crate::asn1::cam::v2::{
    CAM_t,
    asn_DEF_CAM_v2,
    HighFrequencyContainer_PR_HighFrequencyContainer_PR_basicVehicleContainerHighFrequency,
    LowFrequencyContainer_PR_LowFrequencyContainer_PR_basicVehicleContainerLowFrequency,
    uper_decode_complete,
    asn_dec_rval_code_e_RC_OK,
    asn_INTEGER2long,
};
use super::types::{AccelControl, DecodedCam, ExteriorLights};

extern "C" { fn free(ptr: *mut c_void); }

pub fn decode(data: &[u8]) -> Option<DecodedCam> {
    if data.len() < 3 { return None; }
    unsafe {
        let def = std::ptr::addr_of!(asn_DEF_CAM_v2);
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

        // direct free() to avoid symbol conflicts between cam_v1 and cam_v2
        if !cam_ptr.is_null() {
            free(cam_ptr as *mut c_void);
        }

        result
    }
}

unsafe fn enum_to_long(val: *const crate::asn1::cam::v2::INTEGER_t) -> c_long {
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
        result.station_id = header.stationId as u32;
    }

    // v2: cam.cam is *mut CamPayload_t
    let Some(payload) = cam.cam.as_ref() else { return result; };
    result.gen_delta_time_ms = payload.generationDeltaTime as u32;

    let Some(params) = payload.camParameters.as_ref() else { return result; };

    // BasicContainer - v2 uses TrafficParticipantType_t (c_long) and ReferencePositionWithConfidence
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
        }
    }

    // HighFrequencyContainer
    if let Some(hfc) = params.highFrequencyContainer.as_ref() {
        if hfc.present == HighFrequencyContainer_PR_HighFrequencyContainer_PR_basicVehicleContainerHighFrequency {
            let hf_ptr = hfc.choice.basicVehicleContainerHighFrequency;
            if let Some(hf) = hf_ptr.as_ref() {
                if let Some(spd) = hf.speed.as_ref() {
                    let sv = spd.speedValue as i64;
                    if sv < 16383 {
                        result.speed_kmh = Some(sv as f64 * 0.036);
                    }
                }

                if let Some(hdg) = hf.heading.as_ref() {
                    let hv = hdg.headingValue as i64;
                    if hv < 3601 {
                        result.heading_deg = Some(hv as f64 * 0.1);
                    }
                }

                let dd = enum_to_long(&hf.driveDirection as *const _ as *const _);
                if dd < 2 {
                    result.drive_direction = Some(dd as u8);
                }

                if let Some(vl) = hf.vehicleLength.as_ref() {
                    let l = vl.vehicleLengthValue as i64;
                    if l < 1023 {
                        result.vehicle_length_m = Some(l as f64 * 0.1);
                    }
                }

                let w = hf.vehicleWidth as i64;
                if w > 0 && w < 62 {
                    result.vehicle_width_m = Some(w as f64 * 0.1);
                }

                // v2: longitudinalAcceleration is AccelerationComponent_t with value AccelerationValue_t (c_long)
                if let Some(la) = hf.longitudinalAcceleration.as_ref() {
                    let a = la.value as i64;
                    if a != 161 {
                        result.longitudinal_accel = Some(a as f64 * 0.1);
                    }
                }

                if let Some(curv) = hf.curvature.as_ref() {
                    let c = curv.curvatureValue as i32;
                    if c != 30001 {
                        result.curvature = Some(c);
                    }
                }

                if let Some(yr) = hf.yawRate.as_ref() {
                    let y = yr.yawRateValue as i32;
                    if y != 32767 {
                        result.yaw_rate = Some(y as f64 * 0.01);
                    }
                }

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

    // LowFrequencyContainer - same layout as v1
    if let Some(lfc) = params.lowFrequencyContainer.as_ref() {
        if lfc.present == LowFrequencyContainer_PR_LowFrequencyContainer_PR_basicVehicleContainerLowFrequency {
            let lf_ptr = lfc.choice.basicVehicleContainerLowFrequency;
            if let Some(lf) = lf_ptr.as_ref() {
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
