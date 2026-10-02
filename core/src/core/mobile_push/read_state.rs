use super::*;
use std::collections::HashMap;

pub(super) fn dismissed_resolution() -> MobilePushNotificationResolution {
    MobilePushNotificationResolution {
        payload_json: r#"{"iris_dismiss":true}"#.to_string(),
        ..suppressed_resolution()
    }
}

pub(super) fn rumor_is_read(data_dir: &str, chat_id: &str, inner_json: &str) -> bool {
    let Some(rumor) = parse_runtime_rumor(inner_json) else {
        return false;
    };
    if rumor.kind != CHAT_MESSAGE_KIND {
        return false;
    }
    let Some(conn) = open_lookup_connection(data_dir) else {
        return false;
    };
    let state: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [format!("chat_read_state:{chat_id}")],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    state
        .and_then(|json| serde_json::from_str::<ChatReadState>(&json).ok())
        .is_some_and(|state| state.covers(rumor.created_at_secs, &rumor.id))
}

/// Resolve only signed encrypted events. An untrusted push field must never
/// authorize dismissal. Preview decryption uses an overlay and never advances
/// the live ratchet; the foreground runtime owns applying read updates.
pub(crate) fn read_mobile_push_notification_indexes(
    data_dir: String,
    owner_pubkey_hex: String,
    device_nsec: String,
    payloads: Vec<String>,
) -> Vec<u64> {
    let Some(conn) = open_lookup_connection(&data_dir) else {
        return Vec::new();
    };
    if super::super::storage::validate_account_storage(&conn, &owner_pubkey_hex).is_err() {
        return Vec::new();
    }
    drop(conn);
    let mut preview_engine = None;
    let mut dismissed: HashMap<nostr::EventId, bool> = HashMap::new();
    payloads
        .into_iter()
        .enumerate()
        .filter_map(|(index, payload)| {
            let event = mobile_push_event_from_payload(&payload)?;
            if event.kind.as_u16() as u64 != MOBILE_PUSH_OUTER_MESSAGE_EVENT_KIND {
                return None;
            }
            if let Some(is_read) = dismissed.get(&event.id) {
                return (*is_read).then_some(index as u64);
            }
            let event_id = event.id;
            let clean_payload = serde_json::json!({"event": event}).to_string();
            let resolution = decrypt_mobile_push_notification_inner(
                data_dir.clone(),
                owner_pubkey_hex.clone(),
                device_nsec.clone(),
                clean_payload,
                false,
                &mut preview_engine,
            );
            let resolved: serde_json::Value =
                serde_json::from_str(&resolution.payload_json).ok()?;
            let is_read = resolved.get("iris_dismiss").and_then(|value| value.as_bool()) == Some(true);
            dismissed.insert(event_id, is_read);
            is_read.then_some(index as u64)
        })
        .collect()
}
