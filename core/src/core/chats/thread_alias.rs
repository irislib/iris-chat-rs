use super::*;

impl AppCore {
    pub(in crate::core) fn migrate_verified_device_owner_threads(
        &mut self,
        owner: PublicKey,
        app_keys: &AppKeys,
    ) -> bool {
        let owner_hex = owner.to_hex();
        let mut changed = false;
        for device in app_keys.get_all_devices() {
            let device_hex = device.identity_pubkey.to_hex();
            if device_hex != owner_hex {
                changed |= self.migrate_direct_thread_alias(&device_hex, &owner_hex);
            }
        }
        changed
    }

    fn migrate_direct_thread_alias(&mut self, from_chat_id: &str, to_chat_id: &str) -> bool {
        if from_chat_id == to_chat_id || !self.threads.contains_key(from_chat_id) {
            return false;
        }

        let from_label = self.owner_display_label(from_chat_id);
        let to_label = self.owner_display_label(to_chat_id);
        // An inactive thread holds only its newest message. Commit the full
        // durable move before removing either live thread or its fingerprints.
        let migrated = match self.app_store.migrate_direct_thread_history(
            &self.threads[from_chat_id],
            self.threads.get(to_chat_id),
            to_chat_id,
            &from_label,
            &to_label,
        ) {
            Ok(migrated) => migrated,
            Err(error) => {
                self.push_debug_log("storage.chat_alias.error", error.to_string());
                return false;
            }
        };
        let Some(mut source) = self.threads.remove(from_chat_id) else {
            return false;
        };
        let target = self
            .threads
            .entry(to_chat_id.to_string())
            .or_insert_with(|| ThreadRecord {
                chat_id: to_chat_id.to_string(),
                unread_count: 0,
                updated_at_secs: source.updated_at_secs,
                messages: Vec::new(),
                draft: String::new(),
            });
        target
            .messages
            .retain(|message| !migrated.removed_target_loaded_ids.contains(&message.id));

        for mut message in source.messages.drain(..) {
            if migrated.skipped_loaded_ids.contains(&message.id) {
                continue;
            }
            let duplicate = target.messages.iter().any(|existing| {
                existing.id == message.id
                    || message
                        .source_event_id
                        .as_ref()
                        .is_some_and(|source_event_id| {
                            existing.source_event_id.as_ref() == Some(source_event_id)
                        })
            });
            if duplicate {
                continue;
            }
            message.chat_id = to_chat_id.to_string();
            if !message.is_outgoing
                && (message.author == from_chat_id || message.author == from_label)
            {
                message.author = to_label.clone();
            }
            for delivery in &mut message.recipient_deliveries {
                if delivery.owner_pubkey_hex == from_chat_id {
                    delivery.owner_pubkey_hex = to_chat_id.to_string();
                }
            }
            for reactor in &mut message.reactors {
                if reactor.author == from_chat_id {
                    reactor.author = to_chat_id.to_string();
                }
            }
            target.insert_message_sorted(message);
        }
        target.unread_count = migrated.unread_count;
        target.updated_at_secs = migrated.updated_at_secs;
        target.draft = migrated.draft;

        if self.active_chat_id.as_deref() == Some(from_chat_id) {
            self.active_chat_id = Some(to_chat_id.to_string());
        }
        for screen in &mut self.screen_stack {
            if let Screen::Chat { chat_id } = screen {
                if chat_id == from_chat_id {
                    *chat_id = to_chat_id.to_string();
                }
            }
        }
        if let Some(ttl) = self.chat_message_ttl_seconds.remove(from_chat_id) {
            self.chat_message_ttl_seconds
                .entry(to_chat_id.to_string())
                .or_insert(ttl);
        }
        for muted in &mut self.preferences.muted_chat_ids {
            if muted == from_chat_id {
                *muted = to_chat_id.to_string();
            }
        }
        self.preferences.muted_chat_ids.sort();
        self.preferences.muted_chat_ids.dedup();
        for mute in &mut self.preferences.timed_chat_mutes {
            if mute.chat_id == from_chat_id {
                mute.chat_id = to_chat_id.to_string();
            }
        }
        chat_settings::normalize_timed_chat_mutes(&mut self.preferences.timed_chat_mutes);
        for pinned in &mut self.preferences.pinned_chat_ids {
            if pinned == from_chat_id {
                *pinned = to_chat_id.to_string();
            }
        }
        self.preferences.pinned_chat_ids.sort();
        self.preferences.pinned_chat_ids.dedup();
        if let Some(floor) = self.typing_floor_secs.remove(from_chat_id) {
            self.typing_floor_secs
                .entry(to_chat_id.to_string())
                .and_modify(|existing| *existing = (*existing).max(floor))
                .or_insert(floor);
        }
        for peer in self.recent_handshake_peers.values_mut() {
            if peer.owner_hex == from_chat_id {
                peer.owner_hex = to_chat_id.to_string();
            }
        }
        let typing_indicators = std::mem::take(&mut self.typing_indicators);
        for (_, mut indicator) in typing_indicators {
            if indicator.chat_id == from_chat_id {
                indicator.chat_id = to_chat_id.to_string();
            }
            if indicator.author_owner_hex == from_chat_id {
                indicator.author_owner_hex = to_chat_id.to_string();
            }
            self.typing_indicators.insert(
                format!("{}\n{}", indicator.chat_id, indicator.author_owner_hex),
                indicator,
            );
        }
        self.chat_read_states.remove(from_chat_id);
        if let Some(read_state) = migrated.read_state {
            self.chat_read_states
                .insert(to_chat_id.to_string(), read_state);
        }
        self.apply_read_state_to_thread(to_chat_id);
        self.mark_mobile_push_dirty();
        true
    }
}
