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

#[test]
fn typed_record_envelope_size_is_checked_with_real_wire_encoding() {
    let keys = Keys::generate();
    let event = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"name":"Contact", "about":"a".repeat(20_000)}).to_string(),
    )
    .sign_with_keys(&keys)
    .unwrap();
    assert!(DeviceSyncRecord::Profile { event }.fits_packet());
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/deviceSyncRecords.json")).unwrap();
    let mut group: DeviceSyncRecord = serde_json::from_value(
        fixtures
            .into_iter()
            .find(|fixture| fixture["record"]["type"] == "group")
            .unwrap()["record"]
            .clone(),
    )
    .unwrap();
    let DeviceSyncRecord::Group { group: body } = &mut group else {
        unreachable!()
    };
    body.members = vec![keys.public_key().to_hex(); 2_000];
    assert!(!group.fits_packet());
}

#[test]
fn obsolete_message_history_wire_routes_are_not_supported() {
    for value in [
        serde_json::json!({"v":1,"type":"historyMessages","session":"ab".repeat(16),"messages":[],"requested":[]}),
        serde_json::json!({"v":1,"type":"historyPageEnd","linkAt":100,"linkId":"ab".repeat(32)}),
        serde_json::json!({"v":1,"type":"request","rosterAt":100,"page":{"kind":"messages","after":null}}),
        serde_json::json!({"v":1,"type":"historyOpen","session":"ab".repeat(16),"since":100,"until":200,"frame":"00"}),
    ] {
        assert!(serde_json::from_value::<DeviceSyncPacket>(value).is_err());
    }
}
