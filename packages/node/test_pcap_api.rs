use core_logic;
fn main() {
    let expected = std::fs::read_to_string("../core/tests/expected.jsonl").unwrap();
    let first: serde_json::Value = serde_json::from_str(expected.lines().next().unwrap()).unwrap();
    println!("BTP Length: {:?}", first["layers"]["btp"]["btp_btp_length"]);
    println!("ITS payload start bytes: {}", first["layers"]["its"]["its_cam_header"]["its_protocolVersion"]);
    println!("GNW PlLength: {:?}", first["layers"]["gnw"]["gnw_geonw_plLength"]);
}
