use super::types::{DecodedCam, DecodedDenm};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ItsPayload {
    Cam(Box<DecodedCam>),
    Denm(Box<DecodedDenm>),
    Unsupported,
}

pub struct MessageRouter;

impl MessageRouter {
    pub fn decode_payload(btp_port: u16, payload: &[u8]) -> ItsPayload {
        match btp_port {
            2001 => {
                // first byte = protocolVersion (0..255, 8 bits)
                // v1 = EN (protocolVersion 1), v2 = TS (protocolVersion 2)

                let version = payload.first().copied().unwrap_or(0);
                let decoded = match version {
                    2 => super::cam_v2::decode(payload),
                    _ => super::cam_v1::decode(payload),
                };
                match decoded {
                    Some(cam) => ItsPayload::Cam(Box::new(cam)),
                    None => ItsPayload::Unsupported,
                }
            }
            2002 => match super::denm_v1::decode(payload) {
                Some(denm) => ItsPayload::Denm(Box::new(denm)),
                None => ItsPayload::Unsupported,
            },
            _ => ItsPayload::Unsupported,
        }
    }
}
