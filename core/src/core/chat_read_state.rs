use super::*;

impl ChatReadState {
    pub(super) fn exact_messages(&self) -> BTreeMap<String, MessageReadState> {
        let mut reads = self.seen_messages.clone();
        // Older clients only prove the explicitly listed boundary IDs, never
        // every message preceding their timestamp.
        if self.seen_through_secs != 0 {
            for id in &self.seen_at_boundary {
                reads.entry(id.clone()).or_insert(MessageReadState {
                    created_at_secs: self.seen_through_secs,
                    expires_at_secs: None,
                });
            }
        }
        reads
    }

    pub(super) fn includes(&self, other: &Self) -> bool {
        let ours = self.exact_messages();
        (self.updated_at_ms, &self.device_id) >= (other.updated_at_ms, &other.device_id)
            && other
                .exact_messages()
                .iter()
                .all(|(id, read)| ours.get(id) == Some(read))
    }

    pub(super) fn covers(&self, created_at_secs: u64, id: &str) -> bool {
        self.seen_messages
            .get(id)
            .is_some_and(|read| read.created_at_secs == created_at_secs)
            || (self.seen_through_secs != 0
                && created_at_secs == self.seen_through_secs
                && self.seen_at_boundary.contains(id))
    }

    fn merge_seen(&mut self, other: &Self) {
        self.seen_messages.extend(self.exact_messages());
        self.seen_messages.extend(other.exact_messages());
        // Zero legacy fields so older clients cannot turn an exact batch into
        // an implicit "everything before this time" acknowledgement.
        self.seen_through_secs = 0;
        self.seen_at_boundary.clear();
    }

    pub(super) fn bound_recent_batch(&mut self) {
        let mut ids = self
            .seen_messages
            .iter()
            .map(|(id, read)| (read.created_at_secs, id.clone()))
            .collect::<Vec<_>>();
        ids.sort();
        for (_, id) in ids
            .iter()
            .take(ids.len().saturating_sub(MAX_RECENT_READ_IDS))
        {
            self.seen_messages.remove(id);
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
        if message_ids.len() > MAX_RECENT_READ_IDS {
            let mut changed = false;
            for ids in message_ids.chunks(MAX_RECENT_READ_IDS) {
                changed |= self.record_local_chat_read_state(chat_id, ids, force);
            }
            return changed;
        }
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
        let mut next = ChatReadState::default();
        if let Some(thread) = self.threads.get(chat_id) {
            for message in &thread.messages {
                if !message.is_outgoing && message_ids.contains(&message.id) {
                    next.seen_messages.insert(
                        message.id.clone(),
                        MessageReadState {
                            created_at_secs: message.created_at_secs,
                            expires_at_secs: message.expires_at_secs,
                        },
                    );
                }
            }
        }
        if next.seen_messages.is_empty()
            || (!force
                && next.seen_messages.iter().all(|(id, read)| {
                    self.message_was_seen_on_own_device(chat_id, read.created_at_secs, id)
                }))
        {
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

    pub(super) fn chat_read_state_allowed(&self, chat_id: &str, incoming: &ChatReadState) -> bool {
        let max_time = unix_now().get().saturating_add(300);
        if incoming.updated_at_ms == 0
            || incoming.updated_at_ms > max_time.saturating_mul(1000)
            || incoming.seen_through_secs > max_time
            || PublicKey::from_hex(&incoming.device_id).is_err()
            || incoming
                .seen_at_boundary
                .iter()
                .any(|id| id.is_empty() || id.len() > 128)
            || incoming.seen_messages.len() > MAX_RECENT_READ_IDS
            || incoming.seen_messages.iter().any(|(id, read)| {
                id.is_empty()
                    || id.len() > 128
                    || read.created_at_secs > max_time
                    || read
                        .expires_at_secs
                        .is_some_and(|expiry| expiry > 9_007_199_254_740_991)
            })
            || self
                .chat_deletions
                .get(chat_id)
                .is_some_and(|deleted| incoming.updated_at_ms / 1000 <= *deleted)
        {
            return false;
        }
        true
    }

    pub(super) fn apply_chat_read_state(&mut self, chat_id: &str, incoming: ChatReadState) -> bool {
        if !self.chat_read_state_allowed(chat_id, &incoming) {
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
        let reads = incoming
            .exact_messages()
            .into_iter()
            .filter(|(id, read)| {
                read.expires_at_secs
                    .is_none_or(|expiry| expiry > unix_now().get())
                    && !self.chat_activity_is_deleted(chat_id, read.created_at_secs)
                    && !self
                        .app_store
                        .message_was_locally_deleted(chat_id, Some(id), None)
                        .unwrap_or(true)
            })
            .collect::<BTreeMap<_, _>>();
        if reads.is_empty() {
            return false;
        }
        // The cache may evict IDs, but durable per-message status never does.
        merged.merge_seen(&previous);
        merged.merge_seen(&incoming);
        merged.bound_recent_batch();
        if merged == previous
            && reads.iter().all(|(id, read)| {
                self.app_store
                    .message_read_state(chat_id, id)
                    .ok()
                    .flatten()
                    .as_ref()
                    == Some(read)
            })
        {
            return false;
        }
        let unread_count = match self.app_store.save_chat_read_state_batch(
            chat_id,
            &merged,
            &reads,
            self.threads.get(chat_id),
        ) {
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
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        for message in &mut thread.messages {
            if !message.is_outgoing
                && self
                    .app_store
                    .message_read_state(chat_id, &message.id)
                    .ok()
                    .flatten()
                    .is_some_and(|read| read.created_at_secs == message.created_at_secs)
            {
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
        self.app_store
            .message_read_state(chat_id, id)
            .ok()
            .flatten()
            .is_some_and(|read| read.created_at_secs == created_at)
    }
}
