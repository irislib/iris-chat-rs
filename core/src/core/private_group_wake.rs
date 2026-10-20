use super::*;

pub(super) const GROUP_WAKE_KIND: u32 = 21_113;
const MAX_GROUP_WAKE_BYTES: usize = 256 * 1024;
const PREVIEW_ALIAS_PREFIX: &str = "mobile_push_group_wake:";

pub(super) fn preview_source_id(conn: &rusqlite::Connection, event_id: &str) -> String {
    conn.query_row(
        "SELECT value FROM app_meta WHERE key = ?1",
        [format!("{PREVIEW_ALIAS_PREFIX}{event_id}")],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| event_id.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupWake {
    v: u8,
    #[serde(rename = "type")]
    kind: String,
    event: Event,
}

pub(super) fn decode(content: &str) -> Option<Event> {
    if content.len() > MAX_GROUP_WAKE_BYTES {
        return None;
    }
    let wake: GroupWake = serde_json::from_str(content).ok()?;
    if wake.v != 1
        || wake.kind != "group-wake"
        || wake.event.tags.public_keys().next().is_some()
        || parse_group_sender_key_message_event_unchecked(&wake.event).is_err()
    {
        return None;
    }
    Some(wake.event)
}

pub(super) fn sender_matches(engine: &ProtocolEngine, sender: PublicKey, event: &Event) -> bool {
    engine
        .group_sender_event_pubkeys_for_owner(sender)
        .contains(&event.pubkey)
}

impl AppCore {
    pub(super) fn receive_private_group_wake(
        &mut self,
        sender: PublicKey,
        rumor: &RuntimeRumor,
        outer_event_id: Option<String>,
    ) -> Option<bool> {
        if rumor.kind != GROUP_WAKE_KIND {
            return None;
        }
        let Some(event) = decode(&rumor.content) else {
            return Some(true);
        };
        let Some(engine) = self.protocol_engine.as_mut() else {
            return Some(false);
        };
        if !sender_matches(engine, sender, &event) {
            // A distribution may arrive after the wake. Keep that delivery
            // retryable, but discard a known author's wrong sender binding.
            return Some(engine.is_known_group_sender_event_author(event.pubkey));
        }
        let source_id = event.id.to_hex();
        if let Some(wake_id) = outer_event_id.as_deref() {
            if let Err(error) = self.save_group_wake_preview_alias(wake_id, &source_id) {
                self.push_debug_log("group.private_wake.preview", error.to_string());
                return Some(false);
            }
        }
        let Some(engine) = self.protocol_engine.as_mut() else {
            return Some(false);
        };
        let result = match engine.process_group_outer_event(&event) {
            Ok(result) => result,
            Err(error) => {
                self.push_debug_log("group.private_wake.receive", error.to_string());
                return Some(!error.is::<StorageError>());
            }
        };
        let mut applied = true;
        for group_event in result.events {
            self.cache_group_mobile_push_control(&source_id, &group_event);
            applied &=
                self.apply_group_decrypted_event_from_outer(group_event, Some(source_id.clone()));
        }
        self.process_protocol_engine_effects(result.effects);
        self.request_protocol_subscription_refresh();
        Some(applied)
    }

    fn save_group_wake_preview_alias(&self, wake: &str, source: &str) -> anyhow::Result<()> {
        let shared = self.app_store.shared();
        let mut conn = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
            rusqlite::params![format!("{PREVIEW_ALIAS_PREFIX}{wake}"), source],
        )?;
        tx.execute(
            "DELETE FROM app_meta WHERE key IN (
                SELECT key FROM app_meta WHERE key LIKE 'mobile_push_group_wake:%'
                ORDER BY rowid DESC LIMIT -1 OFFSET 4096)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}
