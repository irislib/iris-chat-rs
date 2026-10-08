use super::*;

impl AppCore {
    pub(super) fn group_message_is_hidden(
        &self,
        chat_id: &str,
        message: &ChatMessageSnapshot,
    ) -> bool {
        is_group_chat_id(chat_id)
            && matches!(message.kind, ChatMessageKind::User)
            && !message.is_outgoing
            && message
                .author_owner_pubkey_hex
                .as_deref()
                .is_some_and(|owner| {
                    !self.block_allows_history(chat_id, owner, message.created_at_secs)
                        || (self.preferences.hide_blocked_group_messages
                            && self.is_owner_blocked(owner))
                })
    }

    pub(super) fn message_visibility_revision(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.block_visibility_revision().hash(&mut hash);
        self.preferences.hide_blocked_group_messages.hash(&mut hash);
        self.preferences.blocked_owner_pubkeys.hash(&mut hash);
        hash.finish()
    }

    pub(super) fn blocked_people_snapshot(&self) -> Vec<crate::FollowedUserSearchResult> {
        let mut people = self
            .preferences
            .blocked_owner_pubkeys
            .iter()
            .filter_map(|owner| {
                let key = PublicKey::from_hex(owner).ok()?;
                let user_id = key.to_bech32().ok()?;
                Some(crate::FollowedUserSearchResult {
                    owner_pubkey_hex: key.to_hex(),
                    display_label: self.owner_display_label(owner),
                    profile_label: self.owner_profile_name(owner),
                    user_id,
                    picture_url: None,
                    about: None,
                    social_connection: None,
                })
            })
            .collect::<Vec<_>>();
        people.sort_by(|left, right| {
            left.display_label
                .to_lowercase()
                .cmp(&right.display_label.to_lowercase())
                .then_with(|| left.owner_pubkey_hex.cmp(&right.owner_pubkey_hex))
        });
        people
    }
}
