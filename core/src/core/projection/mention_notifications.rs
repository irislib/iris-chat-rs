use super::*;

impl AppCore {
    pub(super) fn unread_mention_messages(
        &self,
        thread: &ThreadRecord,
    ) -> Vec<crate::state::MentionNotificationSnapshot> {
        if thread.unread_count == 0 || !is_group_chat_id(&thread.chat_id) {
            return vec![];
        }
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|account| account.owner_pubkey.to_hex())
        else {
            return vec![];
        };
        thread
            .messages
            .iter()
            .filter(|message| {
                !message.is_outgoing
                    && !message.deleted_for_everyone
                    && message.kind == ChatMessageKind::User
                    && message.delivery != DeliveryState::Seen
                    && !self.group_message_is_hidden(&thread.chat_id, message)
                    && !message
                        .author_owner_pubkey_hex
                        .as_deref()
                        .is_some_and(|author| self.is_owner_blocked(author))
                    && crate::mentions::mentions_owner(&message.body, &owner)
                    && !self.message_was_seen_on_own_device(
                        &thread.chat_id,
                        message.created_at_secs,
                        &message.id,
                    )
            })
            .map(|message| crate::state::MentionNotificationSnapshot {
                message_id: message.id.clone(),
                preview: message_preview(message),
            })
            .collect()
    }
}
