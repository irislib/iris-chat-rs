use super::*;

impl AppCore {
    pub(in crate::core::device_sync) fn attach_legacy_sync_reactions(
        &self,
        message: &mut DeviceSyncMessage,
    ) {
        let reactors = self
            .threads
            .get(&message.chat_id)
            .and_then(|thread| thread.messages.iter().find(|known| known.id == message.id))
            .map(|known| known.reactors.clone())
            .or_else(|| {
                self.app_store
                    .load_messages_around(&message.chat_id, &message.id, 0, 0)
                    .ok()?
                    .into_iter()
                    .next()
                    .map(|known| known.reactors)
            })
            .unwrap_or_default();
        let reactions = reactors
            .into_iter()
            .filter(|reactor| {
                !self.has_reaction_head(&message.chat_id, &message.id, &reactor.author)
            })
            .take(256)
            .map(|reactor| LegacyReaction {
                author: reactor.author,
                emoji: reactor.emoji,
            })
            .collect::<Vec<_>>();
        message.legacy_reactions = (!reactions.is_empty()).then_some(reactions);
    }
    pub(in crate::core::device_sync) fn apply_legacy_sync_reactions(
        &mut self,
        chat: &str,
        id: &str,
        reactions: Vec<LegacyReaction>,
    ) {
        if reactions.len() > 256 {
            return;
        }
        for reaction in reactions {
            if !reaction.emoji.is_empty()
                && reaction.emoji.len() <= 256
                && self.sync_record_author_allowed(chat, &reaction.author, false)
                && !self.has_reaction_head(chat, id, &reaction.author)
            {
                self.apply_incoming_reaction_to_chat(chat, id, &reaction.author, &reaction.emoji);
            }
        }
    }
    fn has_reaction_head(&self, chat: &str, message: &str, author: &str) -> bool {
        self.sync_head_exists(&serde_json::json!(["reaction", chat, message, author]).to_string())
    }
    pub(in crate::core::device_sync) fn has_group_settings_head(&self, group: &str) -> bool {
        self.sync_head_exists(&serde_json::json!(["groupSettings", group]).to_string())
    }
    fn sync_head_exists(&self, suffix: &str) -> bool {
        let Some(prefix) = self.sync_record_prefix() else {
            return true;
        };
        let shared = self.app_store.shared();
        let Ok(conn) = shared.lock() else { return true };
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_meta WHERE key=?1)",
            [format!("{prefix}{suffix}")],
            |row| row.get(0),
        )
        .unwrap_or(true)
    }
}
