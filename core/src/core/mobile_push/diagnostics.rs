use super::*;

/// Read-only receive diagnostics. The output contains identifiers and outcomes,
/// never message text, secret keys, or serialized ratchet state.
#[uniffi::export]
pub fn diagnose_mobile_push_receive(
    data_dir: String,
    owner_pubkey_hex: String,
    device_nsec: String,
    raw_event_json: String,
) -> String {
    crate::ffi_or(
        "diagnose_mobile_push_receive",
        "{}".to_string(),
        || match diagnose(&data_dir, &owner_pubkey_hex, &device_nsec, &raw_event_json) {
            Ok(value) => value.to_string(),
            Err(error) => serde_json::json!({"error": error.to_string()}).to_string(),
        },
    )
}

fn diagnose(
    data_dir: &str,
    owner: &str,
    device: &str,
    raw: &str,
) -> anyhow::Result<serde_json::Value> {
    let owner = PublicKey::parse(owner)?;
    let device = Keys::parse(device)?;
    let event: Event = serde_json::from_str(raw)?;
    event.verify()?;
    let envelope = nostr_double_ratchet::parse_message_event(&event)?;
    let conn = super::super::storage::open_database(Path::new(data_dir))?;
    let base = Arc::new(super::super::storage::SqliteStorageAdapter::new(
        conn,
        owner.to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let overlay = Arc::new(NotificationPreviewStorage::new(base)) as Arc<dyn StorageAdapter>;
    let mut engine = ProtocolEngine::load_or_create_for_local_device(overlay, owner, &device)?;
    let mut matches = Vec::new();
    for user in engine.session_manager_snapshot().users {
        for record in user.devices {
            for (index, state) in record
                .active_session
                .into_iter()
                .chain(record.inactive_sessions)
                .enumerate()
            {
                let mut session = nostr_double_ratchet::Session::from_state(state);
                if !session.matches_sender(envelope.sender) {
                    continue;
                }
                let mut rng = rand::rngs::OsRng;
                let mut ctx = nostr_double_ratchet::ProtocolContext::new(
                    NdrUnixSeconds(event.created_at.as_secs()),
                    &mut rng,
                );
                let mut inner_kind = None;
                let mut content_length = None;
                let outcome = match session.plan_receive(&mut ctx, &envelope) {
                    Ok(plan) => {
                        let received = session.apply_receive(plan);
                        if let Ok(value) =
                            serde_json::from_slice::<serde_json::Value>(&received.payload)
                        {
                            inner_kind = value.get("kind").and_then(|value| value.as_u64());
                            content_length = value
                                .get("content")
                                .and_then(|value| value.as_str())
                                .map(str::len);
                        }
                        "decryptable".to_string()
                    }
                    Err(error) => error.to_string(),
                };
                matches.push(serde_json::json!({
                    "owner": user.owner_pubkey.to_hex(), "device": record.device_pubkey.to_hex(),
                    "claimed_owner": record.claimed_owner_pubkey.map(|owner| owner.to_hex()),
                    "authorized": record.authorized, "stale": record.is_stale,
                    "session_index": index, "outcome": outcome,
                    "inner_kind": inner_kind, "content_length": content_length,
                }));
            }
        }
    }
    let result = match engine.process_direct_message_event(&event) {
        Ok(Some(message)) => {
            let rumor = serde_json::from_str::<serde_json::Value>(&message.content).ok();
            serde_json::json!({"status": "decrypted", "sender": message.sender.to_hex(),
                "kind": rumor.as_ref().and_then(|value| value.get("kind")),
                "body_length": rumor.as_ref().and_then(|value| value.get("content")).and_then(|value| value.as_str()).map(str::len)})
        }
        Ok(None) => serde_json::json!({"status": "pending"}),
        Err(error) => serde_json::json!({"status": "error", "error": error.to_string()}),
    };
    Ok(
        serde_json::json!({"event_id": event.id.to_hex(), "session_matches": matches, "result": result}),
    )
}
