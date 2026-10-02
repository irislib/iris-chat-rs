use super::*;

impl AppCore {
    pub(in crate::core) fn delete_local_message(&mut self, chat_id: &str, message_id: &str) {
        if chat_id.is_empty() || message_id.is_empty() {
            return;
        }
        let source_event_id = self.threads.get(chat_id).and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .and_then(|message| message.source_event_id.clone())
        });
        // Persist first, including historical rows outside the loaded page. A
        // failed transaction must leave both the visible row and replay intact.
        if let Err(error) =
            self.app_store
                .delete_message_locally(chat_id, message_id, source_event_id.as_deref())
        {
            self.push_debug_log("storage.message.delete.error", error.to_string());
            self.state.toast = Some("Couldn't delete message.".to_string());
            self.emit_state();
            return;
        }
        if let Some(thread) = self.threads.get_mut(chat_id) {
            thread.messages.retain(|message| message.id != message_id);
            thread.updated_at_secs = thread
                .messages
                .last()
                .map(|message| message.created_at_secs)
                .unwrap_or(thread.updated_at_secs);
            if self.active_chat_id.as_deref() == Some(chat_id) {
                thread.unread_count = 0;
            }
        }
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }
}
