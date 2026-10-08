use super::*;

impl AppCore {
    // Heads are committed before projections. Replay them after an interrupted
    // app-state save, without reauthoring or republishing the original controls.
    pub(in crate::core) fn restore_device_sync_record_projection(&mut self) {
        let mut after = String::new();
        loop {
            let Ok(page) = self.sync_record_page(&after) else {
                break;
            };
            if page.is_empty() {
                break;
            }
            for (key, record) in page {
                after = key;
                if !self.sync_record_allowed(&record) {
                    continue;
                }
                match record {
                    DeviceSyncRecord::PrivateBlock { event } => {
                        self.project_private_block(&event);
                    }
                    DeviceSyncRecord::MessageMutation { mutation: m } => {
                        self.project_message_mutations(&m.chat_id, &m.message_id);
                    }
                    DeviceSyncRecord::Reaction { reaction: r } => self
                        .apply_incoming_reaction_to_chat(
                            &r.chat_id,
                            &r.message_id,
                            &r.author,
                            &r.emoji,
                        ),
                    DeviceSyncRecord::GroupSettings { settings: s } => {
                        let chat = group_chat_id(&s.group_id);
                        if let Some(ttl) = s.message_ttl_seconds {
                            self.chat_message_ttl_seconds.insert(chat, ttl);
                        } else {
                            self.chat_message_ttl_seconds.remove(&chat);
                        }
                    }
                    DeviceSyncRecord::Profile { event } => {
                        let projected = self
                            .owner_profiles
                            .get(&event.pubkey.to_hex())
                            .is_some_and(|profile| {
                                profile.source_event_id.as_deref()
                                    == Some(event.id.to_hex().as_str())
                            });
                        if !projected {
                            self.apply_profile_metadata_event(&event);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub(in crate::core) fn restore_device_sync_chat_reactions(&mut self, chat: &str) {
        let ids = self
            .threads
            .get(chat)
            .map(|thread| {
                thread
                    .messages
                    .iter()
                    .map(|message| message.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for id in ids {
            self.restore_device_sync_reactions(chat, &id);
        }
    }

    pub(super) fn sync_group_is_durable(&self, expected: &DeviceSyncGroup) -> bool {
        if self.chat_activity_is_deleted(&group_chat_id(&expected.id), expected.updated_at) {
            return true;
        }
        let shared = self.app_store.shared();
        let Ok(conn) = shared.lock() else {
            return false;
        };
        let json: Result<String, _> = conn.query_row(
            "SELECT group_json FROM groups WHERE group_id=?1",
            [&expected.id],
            |row| row.get(0),
        );
        let Some(group) = json
            .ok()
            .and_then(|json| serde_json::from_str::<GroupSnapshot>(&json).ok())
        else {
            return false;
        };
        let Some(expected) = expected.clone().into_group_snapshot() else {
            return false;
        };
        group == expected
            || (group.revision, group.updated_at) > (expected.revision, expected.updated_at)
    }
}
