use serde::{Deserialize, Serialize};

use crate::asn1::{cam::v1, cam::v2};

/// Enum representing a decoded ASN.1 message from either V1 or V2 standard.
#[derive(Debug)]
pub enum DecodedMessage {
    V1(v1::CAM),
    V2(v2::CAM),
}

/// Attempts to decode UPER encoded ASN.1 data.
/// Automatically detects standard by trying V2 first, then falling back to V1.
pub fn decode_payload(data: &[u8]) -> Option<DecodedMessage> {
    // The actual decoding requires calling the asn1c generated functions,
    // eg. uper_decode_complete(...) which takes the asn_DEF_CAM as an argument.
    // For now: leaving the signature to be implemented properly with the C FFI.

    // TODO: implement C FFI based UPER decode
    
    None
}

impl serde::Serialize for DecodedMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("raw_debug", &format!("{:?}", self))?;
        map.end()
    }
}
