use super::*;

impl AppCore {
    pub(super) fn prune_expired_messages(&mut self, now_secs: u64) -> usize {
        let loaded_removed = self.prune_loaded_expired_messages(now_secs);
        let stored_removed = match self.app_store.delete_expired_messages(now_secs) {
            Ok(deleted) => deleted,
            Err(error) => {
                self.push_debug_log("storage.messages.expire.error", error.to_string());
                0
            }
        };
        self.schedule_next_message_expiry();
        loaded_removed.max(stored_removed)
    }

    pub(super) fn schedule_next_message_expiry(&mut self) {
        self.message_expiry_token = self.message_expiry_token.wrapping_add(1);
        let token = self.message_expiry_token;
        let now = unix_now().get();
        let next_loaded = self.next_loaded_message_expiration_after(now);
        let next_stored = match self.app_store.next_message_expiration_after(now) {
            Ok(expires_at) => expires_at,
            Err(error) => {
                self.push_debug_log("storage.messages.expire.next.error", error.to_string());
                None
            }
        };
        let next = next_loaded.into_iter().chain(next_stored).min();
        let Some(expires_at_secs) = next else {
            return;
        };

        let tx = self.core_sender.clone();
        let delay = Duration::from_secs(expires_at_secs.saturating_sub(now));
        self.runtime.spawn(async move {
            sleep(delay).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::PruneExpiredMessages { token },
            )));
        });
    }

    pub(super) fn handle_prune_expired_messages(&mut self, token: u64) {
        if token != self.message_expiry_token {
            return;
        }
        let removed = self.prune_expired_messages(unix_now().get());
        if removed == 0 {
            return;
        }

        self.rebuild_persist_and_emit_state();
    }

    fn prune_loaded_expired_messages(&mut self, now_secs: u64) -> usize {
        let mut removed = 0;
        for thread in self.threads.values_mut() {
            let expired_unread = thread
                .messages
                .iter()
                .filter(|message| message_is_expired(message, now_secs) && !message.is_outgoing)
                .count() as u64;
            let original_len = thread.messages.len();
            thread
                .messages
                .retain(|message| !message_is_expired(message, now_secs));
            let removed_from_thread = original_len.saturating_sub(thread.messages.len());
            if removed_from_thread == 0 {
                continue;
            }

            removed += removed_from_thread;
            thread.unread_count = thread.unread_count.saturating_sub(expired_unread);
            if let Some(last_message) = thread.messages.last() {
                thread.updated_at_secs = last_message.created_at_secs;
            }
        }
        if removed > 0 {
            self.rebuild_typing_floor_from_threads();
        }
        removed
    }

    fn next_loaded_message_expiration_after(&self, now_secs: u64) -> Option<u64> {
        self.threads
            .values()
            .flat_map(|thread| thread.messages.iter())
            .filter_map(|message| message.expires_at_secs)
            .filter(|expires_at| *expires_at > now_secs)
            .min()
    }

    fn rebuild_typing_floor_from_threads(&mut self) {
        self.typing_floor_secs = self
            .threads
            .iter()
            .filter_map(|(chat_id, thread)| {
                thread
                    .messages
                    .last()
                    .map(|message| (chat_id.clone(), message.created_at_secs))
            })
            .collect();
    }
}

fn message_is_expired(message: &ChatMessageSnapshot, now_secs: u64) -> bool {
    message
        .expires_at_secs
        .is_some_and(|expires_at_secs| expires_at_secs <= now_secs)
}
