use super::*;

pub(super) const GROUP_WAKE_KIND: u32 = 21_113;
const MAX_GROUP_WAKE_BYTES: usize = 256 * 1024;
const PREVIEW_ALIAS_PREFIX: &str = "mobile_push_group_wake:";

// Presence proves that this exact ID was an authenticated ratchet carrier.
// A nested group ciphertext ID alone never supplies that proof.
pub(super) fn preview_source_id(conn: &rusqlite::Connection, event_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM app_meta WHERE key = ?1",
        [format!("{PREVIEW_ALIAS_PREFIX}{event_id}")],
        |row| row.get(0),
    )
    .ok()
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
    /// Group data is one shared ciphertext, which no nearby carrier may take.
    /// While it waits in the outbox, each member with a device on a live
    /// nearby link also gets it inside that member's own ratchet envelope:
    /// when it is queued, and again when a member comes into range. Receivers
    /// drop the copy they see second.
    pub(super) fn send_nearby_group_wakes(&mut self) {
        let pending = self
            .pending_relay_publishes
            .values()
            .filter(|pending| pending.label == GROUP_DATA_PUBLISH_LABEL)
            .filter(|pending| pending.event_json.len() <= MAX_GROUP_WAKE_BYTES / 2)
            .filter_map(|pending| {
                Some((
                    pending.event_id.clone(),
                    pending.chat_id.clone()?,
                    pending.event_json.clone(),
                ))
            })
            .collect::<Vec<_>>();
        self.nearby_group_wakes_sent
            .retain(|(event, _)| pending.iter().any(|(id, _, _)| id == event));
        let Some(local_owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return;
        };
        let local_hex = local_owner.to_hex();
        let nearby_owners = self
            .app_keys
            .iter()
            .filter(|(owner, _)| **owner != local_hex)
            .filter(|(owner, _)| !self.preferences.blocked_owner_pubkeys.contains(*owner))
            .filter(|(_, keys)| {
                keys.devices.iter().any(|device| {
                    self.fips_nearby_links.iter().any(|link| {
                        link.device_pubkey_hex
                            .eq_ignore_ascii_case(&device.identity_pubkey_hex)
                    })
                })
            })
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        if nearby_owners.is_empty() {
            return;
        }
        for (event_id, chat_id, event_json) in pending {
            let Some(members) = parse_group_id_from_chat_id(&chat_id)
                .and_then(|group| self.groups.get(&group))
                .map(|group| {
                    group
                        .members
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
            else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<Event>(&event_json) else {
                continue;
            };
            for owner in nearby_owners
                .iter()
                .filter(|owner| members.iter().any(|member| member == *owner))
            {
                let key = (event_id.clone(), owner.clone());
                if self.nearby_group_wakes_sent.contains(&key) {
                    continue;
                }
                let Ok(peer) = PublicKey::from_hex(owner) else {
                    continue;
                };
                let rumor = UnsignedEvent::new(
                    local_owner,
                    event.created_at,
                    Kind::Custom(GROUP_WAKE_KIND as u16),
                    [],
                    serde_json::json!({"v": 1, "type": "group-wake", "event": event}).to_string(),
                );
                let Some(engine) = self.protocol_engine.as_mut() else {
                    return;
                };
                match engine.send_direct_unsigned_event_to_peer_only(
                    peer,
                    &chat_id,
                    rumor,
                    unix_now(),
                ) {
                    Ok(result) => {
                        self.nearby_group_wakes_sent.insert(key);
                        for ProtocolEffect::Publish(publish) in result.effects {
                            self.publish_protocol_event(publish);
                        }
                    }
                    Err(error) => self.push_debug_log("group.nearby_wake.send", error.to_string()),
                }
            }
        }
    }

    pub(super) fn cache_direct_group_preview_carrier(&mut self, outer_event_id: Option<&str>) {
        let Some(outer_event_id) = outer_event_id else {
            return;
        };
        // An authenticated direct group copy stores its message/control under
        // the carrier's own ID, rather than a nested sender-key ciphertext ID.
        if let Err(error) = self.save_group_wake_preview_alias(outer_event_id, outer_event_id) {
            self.push_debug_log("group.direct.preview", error.to_string());
        }
    }

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
            if engine.is_known_group_sender_event_author(event.pubkey) {
                return Some(true);
            }
            self.request_missing_group_context(sender, None, Some(event.pubkey));
            return Some(false);
        }
        let source_id = event.id.to_hex();
        if let Some(wake_id) = outer_event_id.as_deref() {
            if let Err(error) = self.save_group_wake_preview_alias(wake_id, &source_id) {
                self.push_debug_log("group.private_wake.preview", error.to_string());
                return Some(false);
            }
        }
        Some(self.receive_group_data(&event))
    }

    pub(super) fn receive_group_data(&mut self, event: &Event) -> bool {
        let source_id = event.id.to_hex();
        let Some(engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        let result = match engine.process_group_outer_event(event) {
            Ok(result) => result,
            Err(error) => {
                self.push_debug_log("group.private_wake.receive", error.to_string());
                return !error.is::<StorageError>();
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
        applied
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
