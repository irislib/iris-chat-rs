use super::*;
use nostr_identity::{build_private_setting, parse_private_setting};

fn syncable_target(chat: &str, id: &str) -> bool {
    // allocate_message_id() uses a per-install decimal counter for rows without
    // a protocol ID. The same number can mean a different message on a sibling.
    valid_device_sync_chat_id(chat)
        && !id.is_empty()
        && id.len() <= 128
        && !(id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()))
}

impl AppCore {
    pub(super) fn message_deletion_target(&self, event: &PrivateEvent) -> Option<(String, String)> {
        let (setting, chat, value) =
            parse_private_setting(event, self.logged_in.as_ref()?.owner_pubkey).ok()?;
        let id = value.as_str()?;
        (setting == "messageDeletion" && syncable_target(&chat, id)).then(|| (chat, id.to_owned()))
    }

    pub(in crate::core) fn delete_local_message(&mut self, chat_id: &str, message_id: &str) {
        if chat_id.is_empty() || message_id.is_empty() {
            return;
        }
        if let Err(error) = self.save_message_deletion(chat_id, message_id) {
            self.push_debug_log("storage.message.delete.error", error.to_string());
            self.state.toast = Some("Couldn't delete message.".to_string());
        } else {
            self.persist_best_effort();
            self.rebuild_state();
        }
        self.emit_state();
    }

    fn save_message_deletion(&mut self, chat: &str, id: &str) -> anyhow::Result<()> {
        // Device-local chats and IDs outside the sync protocol remain removable.
        let Some(login) = self
            .logged_in
            .as_ref()
            .filter(|_| syncable_target(chat, id))
        else {
            return self.remove_account_message(chat, id, None);
        };
        let event = build_private_setting(
            login.owner_pubkey,
            "messageDeletion",
            chat,
            &serde_json::json!(id),
        )?;
        self.remove_account_message(
            chat,
            id,
            Some(&DeviceSyncRecord::PrivateMessageDeletion { event }),
        )?;
        self.broadcast_device_sync_snapshot();
        Ok(())
    }

    pub(super) fn remove_account_message(
        &mut self,
        chat_id: &str,
        message_id: &str,
        record: Option<&DeviceSyncRecord>,
    ) -> anyhow::Result<()> {
        let source_event_id = self.threads.get(chat_id).and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|m| m.id == message_id)
                .and_then(|m| m.source_event_id.clone())
        });
        let entry = record.map(|r| self.sync_record_entry(r)).transpose()?;
        let removed_unread = self.app_store.remove_message_for_me(
            chat_id,
            message_id,
            source_event_id.as_deref(),
            self.threads.get(chat_id),
            entry,
        )?;
        if let Some(thread) = self.threads.get_mut(chat_id) {
            thread.messages.retain(|message| message.id != message_id);
            thread.updated_at_secs = thread
                .messages
                .last()
                .map(|message| message.created_at_secs)
                .unwrap_or(thread.updated_at_secs);
            thread.unread_count = thread
                .unread_count
                .saturating_sub(u64::from(removed_unread));
        }
        Ok(())
    }
}
