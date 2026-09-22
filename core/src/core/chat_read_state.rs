use super::*;

impl ChatReadState {
    pub(super) fn covers(&self, created_at_secs: u64, id: &str) -> bool {
        self.seen_through_secs > 0
            && (created_at_secs < self.seen_through_secs
                || (created_at_secs == self.seen_through_secs
                    && self.seen_at_boundary.contains(id)))
    }

    fn merge_seen(&mut self, other: &Self) {
        if other.seen_through_secs > self.seen_through_secs {
            self.seen_through_secs = other.seen_through_secs;
            self.seen_at_boundary = other.seen_at_boundary.clone();
        } else if other.seen_through_secs == self.seen_through_secs {
            self.seen_at_boundary
                .extend(other.seen_at_boundary.iter().cloned());
        }
    }
}

impl AppCore {
    pub(super) fn record_local_chat_read_state(
        &mut self,
        chat_id: &str,
        message_ids: &[String],
        force: bool,
    ) -> bool {
        let Some(local_device) = self
            .logged_in
            .as_ref()
            .map(|state| state.device_keys.public_key().to_hex())
        else {
            return false;
        };
        let previous = self
            .chat_read_states
            .get(chat_id)
            .cloned()
            .unwrap_or_default();
        let mut next = previous.clone();
        if let Some(thread) = self.threads.get(chat_id) {
            for message in &thread.messages {
                if message.is_outgoing || !message_ids.contains(&message.id) {
                    continue;
                }
                if message.created_at_secs > next.seen_through_secs {
                    next.seen_through_secs = message.created_at_secs;
                    next.seen_at_boundary.clear();
                }
                if message.created_at_secs == next.seen_through_secs {
                    next.seen_at_boundary.insert(message.id.clone());
                }
            }
        }
        if force {
            match self.app_store.latest_incoming_read_boundary(chat_id) {
                Ok(Some((created_at, ids))) => {
                    if created_at > next.seen_through_secs {
                        next.seen_through_secs = created_at;
                        next.seen_at_boundary = ids;
                    } else if created_at == next.seen_through_secs {
                        next.seen_at_boundary.extend(ids);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.push_debug_log("storage.chat_read_boundary.error", error.to_string());
                    return false;
                }
            }
        }
        if !force && previous == next && previous.updated_at_ms != 0 {
            return false;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        next.updated_at_ms = now_ms.max(previous.updated_at_ms.saturating_add(1));
        next.device_id = local_device;
        self.apply_chat_read_state(chat_id, next)
    }

    pub(super) fn apply_chat_read_state(&mut self, chat_id: &str, incoming: ChatReadState) -> bool {
        let max_time = unix_now().get().saturating_add(300);
        if incoming.updated_at_ms == 0
            || incoming.updated_at_ms > max_time.saturating_mul(1000)
            || incoming.seen_through_secs > max_time
            || PublicKey::from_hex(&incoming.device_id).is_err()
            || incoming
                .seen_at_boundary
                .iter()
                .any(|id| id.is_empty() || id.len() > 128)
            || self
                .chat_deletions
                .get(chat_id)
                .is_some_and(|deleted| incoming.updated_at_ms / 1000 <= *deleted)
        {
            return false;
        }
        let previous = self
            .chat_read_states
            .get(chat_id)
            .cloned()
            .unwrap_or_default();
        let mut merged = if (incoming.updated_at_ms, &incoming.device_id)
            > (previous.updated_at_ms, &previous.device_id)
        {
            incoming.clone()
        } else {
            previous.clone()
        };
        // Read progress only advances, even when devices reconnect out of order.
        merged.merge_seen(&previous);
        merged.merge_seen(&incoming);
        if merged == previous {
            return false;
        }
        let unread_count =
            match self
                .app_store
                .save_chat_read_state(chat_id, &merged, self.threads.get(chat_id))
            {
                Ok(count) => count,
                Err(error) => {
                    self.push_debug_log("storage.chat_read_state.error", error.to_string());
                    return false;
                }
            };
        self.chat_read_states.insert(chat_id.to_string(), merged);
        self.apply_read_state_to_thread(chat_id);
        if let (Some(thread), Some(unread_count)) = (self.threads.get_mut(chat_id), unread_count) {
            thread.unread_count = unread_count;
        }
        true
    }

    pub(super) fn apply_read_state_to_thread(&mut self, chat_id: &str) {
        let Some(state) = self.chat_read_states.get(chat_id) else {
            return;
        };
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        for message in &mut thread.messages {
            if !message.is_outgoing && state.covers(message.created_at_secs, &message.id) {
                message.delivery = DeliveryState::Seen;
            }
        }
    }

    pub(super) fn apply_own_read_state_tag(&mut self, chat_id: &str, tags: &[nostr::Tag]) {
        let Some(encoded) = first_tag_value(tags.iter(), "iris-read-state") else {
            return;
        };
        let Ok(state) = serde_json::from_str::<ChatReadState>(&encoded) else {
            return;
        };
        self.apply_chat_read_state(chat_id, state);
    }

    pub(super) fn message_was_seen_on_own_device(
        &self,
        chat_id: &str,
        created_at: u64,
        id: &str,
    ) -> bool {
        self.chat_read_states
            .get(chat_id)
            .is_some_and(|state| state.covers(created_at, id))
    }
}
