use super::*;
use base64::Engine;

#[test]
fn typed_record_ids_match_browser_fixture() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/deviceSyncRecords.json")).unwrap();
    for fixture in fixtures {
        let mut wire = fixture["record"].clone();
        if wire["type"] == "message" {
            wire["message"]["body"] = base64::engine::general_purpose::STANDARD
                .encode(wire["message"]["body"].as_str().unwrap())
                .into();
        }
        let record: DeviceSyncRecord = serde_json::from_value(wire).unwrap();
        let id = record
            .id()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(id, fixture["id"], "{}", fixture["name"]);
        assert_eq!(record.timestamp(), fixture["timestamp"].as_u64().unwrap());
        if let DeviceSyncRecord::Profile { event } = record {
            event.verify().unwrap();
        }
    }
}
