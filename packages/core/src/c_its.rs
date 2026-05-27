use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItsProtocol {
    Cam,
    Denm,
    Mapem,
    Spatem,
    Ivim,
    Srem,
    Ssem,
    Cpm,
    GeoNw,
    Btp,
}

impl ItsProtocol {
    pub fn btp_port(&self) -> Option<u16> {
        match self {
            ItsProtocol::Cam => Some(2001),
            ItsProtocol::Denm => Some(2002),
            ItsProtocol::Mapem => Some(2003),
            ItsProtocol::Spatem => Some(2004),
            ItsProtocol::Ivim => Some(2006),
            ItsProtocol::Srem => Some(2007),
            ItsProtocol::Ssem => Some(2008),
            ItsProtocol::Cpm => Some(2009),
            _ => None,
        }
    }

    /// returns the ETSI ITS-G5 mesage ID for this protocol
    pub fn msg_id(&self) -> Option<u8> {
        match self {
            ItsProtocol::Cam => Some(2),
            ItsProtocol::Denm => Some(1),
            ItsProtocol::Mapem => Some(5),
            ItsProtocol::Spatem => Some(4),
            ItsProtocol::Ivim => Some(6),
            ItsProtocol::Srem => Some(9),
            ItsProtocol::Ssem => Some(10),
            ItsProtocol::Cpm => Some(14),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum StationType {
    Unknown = 0,
    Pedestrian = 1,
    Cyclist = 2,
    Moped = 3,
    Motorcycle = 4,
    PassengerCar = 5,
    Bus = 6,
    LightTruck = 7,
    HeavyTruck = 8,
    Trailer = 9,
    SpecialVehicles = 10,
    Tram = 11,
    LightVruVehicle = 12,
    Animal = 13,
    RoadSideUnit = 15,
}

impl From<u8> for StationType {
    fn from(val: u8) -> Self {
        match val {
            1 => StationType::Pedestrian,
            2 => StationType::Cyclist,
            3 => StationType::Moped,
            4 => StationType::Motorcycle,
            5 => StationType::PassengerCar,
            6 => StationType::Bus,
            7 => StationType::LightTruck,
            8 => StationType::HeavyTruck,
            9 => StationType::Trailer,
            10 => StationType::SpecialVehicles,
            11 => StationType::Tram,
            12 => StationType::LightVruVehicle,
            13 => StationType::Animal,
            15 => StationType::RoadSideUnit,
            _ => StationType::Unknown,
        }
    }
}
