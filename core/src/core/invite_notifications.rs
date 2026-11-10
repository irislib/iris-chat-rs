use super::*;

impl AppCore {
    pub(super) fn record_new_chat_notification(&mut self, chat_id: &str, event_id: &str) {
        if self.threads.contains_key(chat_id)
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey.to_hex() == chat_id)
            || self.direct_request_access(chat_id) == message_requests::RequestAccess::Denied
        {
            return;
        }
        self.ensure_thread_record(chat_id, unix_now().get());
        if self.direct_request_access(chat_id) != message_requests::RequestAccess::Unconnected
            && self.preferences.desktop_notifications_enabled
            && self.preferences.invite_acceptance_notifications_enabled
            && !self.is_chat_muted(chat_id)
        {
            self.new_chat_notifications
                .insert(chat_id.to_owned(), event_id.to_owned());
        }
    }
}
