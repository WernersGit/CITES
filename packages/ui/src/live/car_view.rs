use dioxus::prelude::*;
use crate::trajectory::DriveDirection;
use super::VehicleState;

fn headlight_css(s: &VehicleState) -> &'static str {
    if s.no_light || (!s.daytime_running && !s.low_beam && !s.high_beam) {
        "headlight off"
    } else if s.high_beam {
        "headlight high-beam"
    } else if s.low_beam {
        "headlight low-beam"
    } else {
        "headlight off"
    }
}

fn turn_css(active: bool) -> &'static str {
    if active { "turn-signal blinking" } else { "turn-signal" }
}

fn tail_css(brake: bool) -> &'static str {
    if brake { "tail-light brake" } else { "tail-light" }
}

fn accel_css(active: bool) -> &'static str {
    if active { "accel-line active" } else { "accel-line" }
}

/// top-down SVG car view with speed and ADAS indicatros
// TODO: add night-mode styling
#[component]
pub fn CarView(state: VehicleState) -> Element {
    rsx! {
        CarVisualization { state: state.clone() }
        SpeedDisplay { speed_kmh: state.speed_kmh }
        AdasIndicators { state }
    }
}

#[component]
fn CarVisualization(state: VehicleState) -> Element {
    let hl  = headlight_css(&state);
    let tl  = tail_css(state.brake);
    let lsig = turn_css(state.blink_left());
    let rsig = turn_css(state.blink_right());
    let acl  = accel_css(state.accelerating);
    let drl =
        !state.no_light && (state.daytime_running || state.low_beam || state.high_beam);

    let forward  = state.drive_direction == Some(DriveDirection::Forward);
    let backward = state.drive_direction == Some(DriveDirection::Backward);

    rsx! {
        div { class: "car-svg-container",
            svg {
                view_box: "0 -44 200 448",
                xmlns: "http://www.w3.org/2000/svg",
                class: "car-svg",

                // direction arrow

                // forward arrow: tip points away from car (up), base near car front
                if forward {
                    polygon {
                        class: "direction-arrow",
                        points: "100,-38 78,-8 122,-8",
                    }
                }

                // backward arrow: tip points away from car (down), base with clearance below rear
                if backward {
                    polygon {
                        class: "direction-arrow",
                        points: "100,398 78,368 122,368",
                    }
                }

                // speed line
                g { class: "{acl} accel-left",
                    line {
                        class: "speed-line speed-line-1",
                        x1: "24",
                        y1: "175",
                        x2: "-14",
                        y2: "182",
                    }
                    line {
                        class: "speed-line speed-line-2",
                        x1: "24",
                        y1: "195",
                        x2: "-18",
                        y2: "201",
                    }
                    line {
                        class: "speed-line speed-line-3",
                        x1: "24",
                        y1: "215",
                        x2: "-10",
                        y2: "220",
                    }
                }
                g { class: "{acl} accel-right",
                    line {
                        class: "speed-line speed-line-1",
                        x1: "176",
                        y1: "175",
                        x2: "214",
                        y2: "182",
                    }
                    line {
                        class: "speed-line speed-line-2",
                        x1: "176",
                        y1: "195",
                        x2: "218",
                        y2: "201",
                    }
                    line {
                        class: "speed-line speed-line-3",
                        x1: "176",
                        y1: "215",
                        x2: "210",
                        y2: "220",
                    }
                }

                // Wheel
                rect {
                    class: "wheel",
                    x: "4",
                    y: "82",
                    width: "26",
                    height: "58",
                    rx: "6",
                }
                rect {
                    class: "wheel",
                    x: "170",
                    y: "82",
                    width: "26",
                    height: "58",
                    rx: "6",
                }
                rect {
                    class: "wheel",
                    x: "4",
                    y: "212",
                    width: "26",
                    height: "58",
                    rx: "6",
                }
                rect {
                    class: "wheel",
                    x: "170",
                    y: "212",
                    width: "26",
                    height: "58",
                    rx: "6",
                }
                rect {
                    class: "wheel-rim",
                    x: "8",
                    y: "88",
                    width: "18",
                    height: "46",
                    rx: "4",
                }
                rect {
                    class: "wheel-rim",
                    x: "174",
                    y: "88",
                    width: "18",
                    height: "46",
                    rx: "4",
                }
                rect {
                    class: "wheel-rim",
                    x: "8",
                    y: "218",
                    width: "18",
                    height: "46",
                    rx: "4",
                }
                rect {
                    class: "wheel-rim",
                    x: "174",
                    y: "218",
                    width: "18",
                    height: "46",
                    rx: "4",
                }

                //  body
                rect {
                    class: "car-body",
                    x: "28",
                    y: "12",
                    width: "144",
                    height: "336",
                    rx: "22",
                }
                rect {
                    class: "car-hood",
                    x: "28",
                    y: "12",
                    width: "144",
                    height: "80",
                    rx: "22",
                }
                rect {
                    class: "car-trunk",
                    x: "28",
                    y: "268",
                    width: "144",
                    height: "80",
                    rx: "22",
                }

                // windsheilds
                rect {
                    class: "windshield front-windshield",
                    x: "42",
                    y: "90",
                    width: "116",
                    height: "58",
                    rx: "8",
                }
                rect {
                    class: "windshield rear-windshield",
                    x: "42",
                    y: "208",
                    width: "116",
                    height: "55",
                    rx: "8",
                }

                // roof / cabi
                rect {
                    class: "cabin",
                    x: "40",
                    y: "148",
                    width: "120",
                    height: "60",
                    rx: "5",
                }

                // Pillars
                line {
                    class: "pillar",
                    x1: "42",
                    y1: "148",
                    x2: "52",
                    y2: "92",
                }
                line {
                    class: "pillar",
                    x1: "158",
                    y1: "148",
                    x2: "148",
                    y2: "92",
                }
                line {
                    class: "pillar",
                    x1: "42",
                    y1: "208",
                    x2: "52",
                    y2: "263",
                }
                line {
                    class: "pillar",
                    x1: "158",
                    y1: "208",
                    x2: "148",
                    y2: "263",
                }

                // centre line
                line {
                    class: "body-line",
                    x1: "100",
                    y1: "12",
                    x2: "100",
                    y2: "88",
                }
                line {
                    class: "body-line",
                    x1: "100",
                    y1: "268",
                    x2: "100",
                    y2: "346",
                }

                // mirrors
                rect {
                    class: "mirror",
                    x: "12",
                    y: "98",
                    width: "16",
                    height: "22",
                    rx: "4",
                }
                rect {
                    class: "mirror",
                    x: "172",
                    y: "98",
                    width: "16",
                    height: "22",
                    rx: "4",
                }

                // front turn signals
                rect {
                    class: "{lsig}",
                    x: "28",
                    y: "14",
                    width: "52",
                    height: "16",
                    rx: "8",
                }
                rect {
                    class: "{rsig}",
                    x: "120",
                    y: "14",
                    width: "52",
                    height: "16",
                    rx: "8",
                }

                // front headlights
                rect {
                    class: "{hl}",
                    x: "28",
                    y: "32",
                    width: "58",
                    height: "18",
                    rx: "5",
                }
                rect {
                    class: "{hl}",
                    x: "114",
                    y: "32",
                    width: "58",
                    height: "18",
                    rx: "5",
                }

                // DRL accent strips
                if drl {
                    rect {
                        class: "drl-strip",
                        x: "38",
                        y: "52",
                        width: "36",
                        height: "5",
                        rx: "2",
                    }
                    rect {
                        class: "drl-strip",
                        x: "126",
                        y: "52",
                        width: "36",
                        height: "5",
                        rx: "2",
                    }
                }

                //high-beam projection rays
                if state.high_beam {
                    line {
                        class: "beam-ray",
                        x1: "48",
                        y1: "32",
                        x2: "18",
                        y2: "5",
                    }
                    line {
                        class: "beam-ray",
                        x1: "60",
                        y1: "32",
                        x2: "42",
                        y2: "2",
                    }
                    line {
                        class: "beam-ray",
                        x1: "72",
                        y1: "32",
                        x2: "68",
                        y2: "0",
                    }
                    line {
                        class: "beam-ray",
                        x1: "128",
                        y1: "32",
                        x2: "132",
                        y2: "5",
                    }
                    line {
                        class: "beam-ray",
                        x1: "140",
                        y1: "32",
                        x2: "158",
                        y2: "2",
                    }
                    line {
                        class: "beam-ray",
                        x1: "152",
                        y1: "32",
                        x2: "182",
                        y2: "0",
                    }
                }

                // grille
                line {
                    class: "grille",
                    x1: "68",
                    y1: "60",
                    x2: "132",
                    y2: "60",
                }
                line {
                    class: "grille",
                    x1: "72",
                    y1: "67",
                    x2: "128",
                    y2: "67",
                }

                // rear tail / brake lights
                rect {
                    class: "{tl}",
                    x: "28",
                    y: "308",
                    width: "58",
                    height: "18",
                    rx: "5",
                }
                rect {
                    class: "{tl}",
                    x: "114",
                    y: "308",
                    width: "58",
                    height: "18",
                    rx: "5",
                }

                // rear trun signals
                rect {
                    class: "{lsig}",
                    x: "28",
                    y: "328",
                    width: "52",
                    height: "16",
                    rx: "8",
                }
                rect {
                    class: "{rsig}",
                    x: "120",
                    y: "328",
                    width: "52",
                    height: "16",
                    rx: "8",
                }

                // rear centre strip
                rect {
                    class: "rear-center",
                    x: "76",
                    y: "343",
                    width: "48",
                    height: "7",
                    rx: "3",
                }
            }
        }
    }
}

#[component]
fn SpeedDisplay(speed_kmh: Option<f64>) -> Element {
    rsx! {
        div { class: "car-speed",
            if let Some(s) = speed_kmh {
                "{s:.1} km/h"
            } else {
                "— km/h"
            }
        }
    }
}

#[component]
fn AdasIndicators(state: VehicleState) -> Element {
    rsx! {
        div { class: "car-adas-row",
            span { class: if state.acc_engaged { "adas-badge active" } else { "adas-badge" }, "ACC" }
            span { class: if state.cruise_control_active { "adas-badge active" } else { "adas-badge" },
                "CC"
            }
            span { class: if state.speed_limiter_active { "adas-badge active" } else { "adas-badge" },
                "SL"
            }
        }
    }
}
