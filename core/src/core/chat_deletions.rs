use super::*;

impl AppCore {
    pub(super) fn clear_chats_and_deletions(&mut self) {
        self.threads.clear();
        self.chat_deletions.clear();
    }

    pub(super) fn chat_activity_is_deleted(&self, chat_id: &str, created_at: u64) -> bool {
        self.chat_deletions
            .get(chat_id)
            .is_some_and(|deleted_at| created_at <= *deleted_at)
    }

    // Explicitly reopening or sending starts after the deletion, even within
    // the same wire-clock second. Keep the cutoff to reject older history.
    pub(super) fn chat_activity_after_deletion(&self, chat_id: &str, now: u64) -> u64 {
        self.chat_deletions
            .get(chat_id)
            .map_or(now, |at| now.max(at.saturating_add(1)))
    }

    pub(super) fn delete_chat(&mut self, chat_id: &str) {
        let Some(chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let group_id = parse_group_id_from_chat_id(&chat_id);
        if !self.threads.contains_key(&chat_id)
            && !group_id
                .as_ref()
                .is_some_and(|id| self.groups.contains_key(id))
        {
            return;
        }
        let deleted_at = self
            .threads
            .get(&chat_id)
            .map_or(unix_now().get(), |thread| {
                unix_now().get().max(thread.updated_at_secs)
            })
            .max(
                group_id
                    .as_ref()
                    .and_then(|id| self.groups.get(id))
                    .map_or(0, |group| group.updated_at.get()),
            );
        if self.apply_chat_deletion(&chat_id, deleted_at) {
            self.push_debug_log("chat.delete", chat_id);
            self.rebuild_persist_and_emit_state();
            self.broadcast_device_sync_snapshot();
        }
    }

    pub(super) fn apply_chat_deletion(&mut self, chat_id: &str, deleted_at: u64) -> bool {
        if self
            .chat_deletions
            .get(chat_id)
            .is_some_and(|at| *at >= deleted_at)
        {
            return false;
        }
        let keep_thread = self
            .threads
            .get(chat_id)
            .is_some_and(|thread| thread.updated_at_secs > deleted_at);
        let group_id = parse_group_id_from_chat_id(chat_id);
        let keep_group = keep_thread
            || group_id.as_ref().is_some_and(|id| {
                self.groups
                    .get(id)
                    .is_some_and(|group| group.updated_at.get() > deleted_at)
            });
        // The durable cutoff and history removal must commit together before
        // we notify siblings, otherwise a crash could resurrect deleted chats.
        if let Err(error) =
            self.app_store
                .apply_chat_deletion(chat_id, deleted_at, keep_thread, keep_group)
        {
            self.push_debug_log("storage.chat_delete.error", error.to_string());
            return false;
        }
        self.chat_deletions.insert(chat_id.to_string(), deleted_at);
        if keep_thread {
            if let Some(thread) = self.threads.get_mut(chat_id) {
                thread
                    .messages
                    .retain(|message| message.created_at_secs > deleted_at);
                thread.unread_count = thread.unread_count.min(thread.messages.len() as u64);
            }
        } else {
            self.threads.remove(chat_id);
            self.chat_message_ttl_seconds.remove(chat_id);
            self.preferences.muted_chat_ids.retain(|id| id != chat_id);
            self.preferences.pinned_chat_ids.retain(|id| id != chat_id);
            self.typing_indicators
                .retain(|_, indicator| indicator.chat_id != chat_id);
            self.typing_floor_secs.remove(chat_id);
            if self.active_chat_id.as_deref() == Some(chat_id) {
                self.active_chat_id = None;
            }
            self.screen_stack.retain(|screen| match screen {
                Screen::Chat { chat_id: id } | Screen::DirectChatInfo { chat_id: id } => {
                    id != chat_id
                }
                Screen::GroupDetails { group_id: id } => group_id.as_ref() != Some(id),
                _ => true,
            });
        }
        if !keep_group {
            if let Some(id) = group_id {
                self.groups.remove(&id);
                self.group_pictures.remove(&id);
                self.sync_runtime_groups();
            }
        }
        self.mark_mobile_push_dirty();
        true
    }
}
