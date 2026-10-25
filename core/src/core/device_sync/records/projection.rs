use super::*;
use anyhow::Context;

impl AppCore {
    // Heads are committed before projections. Replay them after an interrupted
    // app-state save, without reauthoring or republishing the original controls.
    pub(in crate::core) fn restore_device_sync_record_projection(&mut self) -> anyhow::Result<()> {
        // Only app projection/persistence is coalesced here. Protocol commits
        // keep their existing ordering, including when no outer batch exists.
        // The caller owns the completed-state emission after this checked save.
        let previous_depth = self.batch_depth;
        let previous_state = self.batch_dirty_state;
        let previous_persist = self.batch_dirty_persist;
        self.batch_depth = previous_depth
            .checked_add(1)
            .context("restore projection batch depth overflow")?;
        let result = self
            .restore_device_sync_record_projection_inner()
            .and_then(|()| {
                self.persist_app_state()
                    .context("persist restored device-sync projection")
            });
        self.batch_depth = previous_depth;
        self.batch_dirty_state = previous_state;
        self.batch_dirty_persist = if result.is_ok() {
            false
        } else {
            previous_persist
        };
        result
    }

    fn restore_device_sync_record_projection_inner(&mut self) -> anyhow::Result<()> {
        let mut after = String::new();
        loop {
            let page = self
                .sync_record_page(&after)
                .context("restore device-sync record page")?;
            if page.is_empty() {
                break;
            }
            for (key, record) in page {
                after = key;
                if !self.sync_record_allowed(&record) {
                    continue;
                }
                if matches!(
                    record,
                    DeviceSyncRecord::PrivateBlock { .. }
                        | DeviceSyncRecord::PrivateNameApproval { .. }
                ) {
                    // Verify legacy signatures on read, then retire them durably.
                    if self.store_sync_record(&record).is_err() {
                        continue;
                    }
                }
                match record {
                    DeviceSyncRecord::Authorization { event } => {
                        self.project_authorization_event(&event)
                            .context("restore authorization projection")?;
                    }
                    DeviceSyncRecord::PrivateEvent { event } => {
                        if let Some(state) =
                            super::super::private_settings::request_decision_state(&event)
                        {
                            self.project_request_decision(&state);
                        }
                        if let Some(state) =
                            super::super::private_settings::mention_notifications::state(&event)
                        {
                            self.project_mention_notifications(&state);
                        }
                    }
                    DeviceSyncRecord::PrivateNameApproval { event } => {
                        self.project_name_approval(&event)
                    }
                    DeviceSyncRecord::PrivateBlock { event } => {
                        self.project_private_block(&event);
                    }
                    DeviceSyncRecord::MessageMutation { mutation: m } => {
                        self.project_message_mutations(&m.chat_id, &m.message_id);
                    }
                    DeviceSyncRecord::Reaction { reaction: r } => {
                        self.observe_chat_engagement(
                            &r.chat_id,
                            &r.author,
                            effective_ms(r.created_at, r.created_at_ms),
                        );
                        self.apply_incoming_reaction_to_chat(
                            &r.chat_id,
                            &r.message_id,
                            &r.author,
                            &r.emoji,
                        );
                    }
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
        // Upgrade already-loaded history from clients that only projected the
        // request banner locally. Use the original time, never restart time.
        if let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        {
            let engagement = self
                .threads
                .values()
                .filter_map(|thread| {
                    thread
                        .messages
                        .iter()
                        .filter(|message| {
                            message.is_outgoing
                                || message.reactors.iter().any(|reaction| {
                                    reaction.author == owner
                                        && !self.has_reaction_head(
                                            &thread.chat_id,
                                            &message.id,
                                            &owner,
                                        )
                                })
                        })
                        .map(|message| message.created_at_secs)
                        .max()
                        .map(|time| (thread.chat_id.clone(), time))
                })
                .collect::<Vec<_>>();
            for (chat, time) in engagement {
                self.observe_chat_engagement(&chat, &owner, time.saturating_mul(1000));
            }
        }
        Ok(())
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
