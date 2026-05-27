/// A European country with pre-computed map center and zoom level.
pub struct Country {
    pub code: &'static str,
    pub name: &'static str,
    /// Center longitude (GeoJSON / MapLibre order: lon before lat).
    pub lon: f64,
    /// Center latitude.
    pub lat: f64,
    /// Default zoom showing the full country.
    pub zoom: u8,
}

pub static COUNTRIES: &[Country] = &[
    Country { code: "AT", name: "Austria",         lon:  14.12, lat: 47.60, zoom: 6  },
    Country { code: "BE", name: "Belgium",          lon:   4.47, lat: 50.50, zoom: 7  },
    Country { code: "BG", name: "Bulgaria",         lon:  25.50, lat: 42.70, zoom: 7  },
    Country { code: "CH", name: "Switzerland",      lon:   8.23, lat: 46.82, zoom: 7  },
    Country { code: "CY", name: "Cyprus",           lon:  33.43, lat: 35.13, zoom: 8  },
    Country { code: "CZ", name: "Czech Republic",   lon:  15.47, lat: 49.82, zoom: 7  },
    Country { code: "DE", name: "Germany",          lon:  10.45, lat: 51.20, zoom: 5  },
    Country { code: "DK", name: "Denmark",          lon:  10.00, lat: 56.00, zoom: 6  },
    Country { code: "EE", name: "Estonia",          lon:  24.75, lat: 58.60, zoom: 7  },
    Country { code: "ES", name: "Spain",            lon:  -3.70, lat: 40.40, zoom: 5  },
    Country { code: "FI", name: "Finland",          lon:  25.00, lat: 64.00, zoom: 5  },
    Country { code: "FR", name: "France",           lon:   2.35, lat: 46.23, zoom: 5  },
    Country { code: "GR", name: "Greece",           lon:  21.82, lat: 39.07, zoom: 6  },
    Country { code: "HR", name: "Croatia",          lon:  15.20, lat: 45.10, zoom: 7  },
    Country { code: "HU", name: "Hungary",          lon:  19.50, lat: 47.16, zoom: 7  },
    Country { code: "IE", name: "Ireland",          lon:  -8.24, lat: 53.41, zoom: 7  },
    Country { code: "IT", name: "Italy",            lon:  12.57, lat: 41.87, zoom: 5  },
    Country { code: "LT", name: "Lithuania",        lon:  23.88, lat: 55.17, zoom: 7  },
    Country { code: "LU", name: "Luxembourg",       lon:   6.13, lat: 49.77, zoom: 9  },
    Country { code: "LV", name: "Latvia",           lon:  24.75, lat: 56.85, zoom: 7  },
    Country { code: "MT", name: "Malta",            lon:  14.37, lat: 35.90, zoom: 10 },
    Country { code: "NL", name: "Netherlands",      lon:   5.29, lat: 52.13, zoom: 7  },
    Country { code: "NO", name: "Norway",           lon:   8.47, lat: 60.47, zoom: 5  },
    Country { code: "PL", name: "Poland",           lon:  19.14, lat: 51.92, zoom: 6  },
    Country { code: "PT", name: "Portugal",         lon:  -8.22, lat: 39.40, zoom: 6  },
    Country { code: "RO", name: "Romania",          lon:  24.97, lat: 45.94, zoom: 7  },
    Country { code: "SE", name: "Sweden",           lon:  15.00, lat: 62.00, zoom: 5  },
    Country { code: "SI", name: "Slovenia",         lon:  14.99, lat: 46.12, zoom: 8  },
    Country { code: "SK", name: "Slovakia",         lon:  19.70, lat: 48.67, zoom: 7  },
];

/// Returns the country matching `code`, falling back to Germany.
pub fn find_country(code: &str) -> &'static Country {
    COUNTRIES
        .iter()
        .find(|c| c.code == code)
        .or_else(|| COUNTRIES.iter().find(|c| c.code == "DE"))
        .unwrap_or(&COUNTRIES[0])
}
