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
            Self::Cam => Some(2001),
            Self::Denm => Some(2002),
            Self::Mapem => Some(2003),
            Self::Spatem => Some(2004),
            Self::Ivim => Some(2006),
            Self::Srem => Some(2007),
            Self::Ssem => Some(2008),
            Self::Cpm => Some(2009),
            _ => None,
        }
    }
}
