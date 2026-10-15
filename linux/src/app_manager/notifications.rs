use super::*;

impl AppManager {
    pub fn notification_message_target(
        &self,
        chat_id: &str,
        message_id: Option<String>,
    ) -> Option<(String, crate::platform::notifications::NotificationTarget)> {
        let state = self.local_state.borrow();
        let account = state.account.as_ref()?;
        if let Some(message_id) = message_id.as_ref() {
            let message = iris_chat_core::DesktopNotificationMessage {
                chat_id: chat_id.into(),
                message_id: message_id.clone(),
            };
            if !iris_chat_core::read_desktop_notification_indexes(
                self.data_dir.to_string_lossy().into_owned(),
                account.public_key_hex.clone(),
                vec![message],
            )
            .is_empty()
            {
                return None;
            }
        }
        Some(self.notification_routing.borrow_mut().message_target(
            &account.public_key_hex,
            chat_id,
            message_id,
        ))
    }

    pub fn clear_read_notifications(&self, state: &AppState) {
        if let Some(account) = state.account.as_ref() {
            let read = state
                .chat_list
                .iter()
                .filter(|chat| chat.unread_count == 0)
                .map(|chat| chat.chat_id.clone())
                .collect();
            self.notification_routing.borrow_mut().clear_read(
                &self.data_dir,
                &account.public_key_hex,
                &read,
            );
        }
    }
}
