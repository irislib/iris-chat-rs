use super::*;

impl AppCore {
    pub(super) fn toggle_reaction(&mut self, chat_id: &str, message_id: &str, emoji: &str) {
        let emoji = emoji.trim();
        if chat_id.is_empty() || message_id.is_empty() || emoji.is_empty() {
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(thread) = self.threads.get_mut(&normalized_chat_id) else {
            return;
        };
        let Some(message) = thread
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        else {
            return;
        };
        toggle_local_reaction(message, emoji);
        self.send_reaction(&normalized_chat_id, message_id, emoji);
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn send_reaction(&mut self, chat_id: &str, message_id: &str, emoji: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if let Some(group_id) = parse_group_id_from_chat_id(chat_id) {
            let mut outer_events = Vec::new();
            let event = GroupSendEvent {
                kind: REACTION_KIND,
                content: emoji.to_string(),
                tags: vec![vec!["e".to_string(), message_id.to_string()]],
            };
            let mut result = Ok(());
            logged_in
                .ndr_runtime
                .with_group_context(|session_manager, group_manager, _| {
                    let mut send_pairwise = |recipient: PublicKey, rumor: &UnsignedEvent| {
                        session_manager
                            .send_event(recipient, rumor.clone())
                            .map(|_| ())
                    };
                    let mut publish_outer = |event: &Event| {
                        outer_events.push(event.clone());
                        Ok(())
                    };
                    result = group_manager
                        .send_event(
                            &group_id,
                            event,
                            &mut send_pairwise,
                            &mut publish_outer,
                            None,
                        )
                        .map(|_| ());
                });
            if result.is_ok() {
                for event in outer_events {
                    self.publish_runtime_event(event, "group reaction", None);
                }
                self.process_runtime_events();
            }
            return;
        }

        if let Ok((_, peer)) = parse_peer_input(chat_id) {
            let _ = logged_in.ndr_runtime.send_reaction(
                peer,
                message_id.to_string(),
                emoji.to_string(),
                None,
            );
            self.process_runtime_events();
        }
    }

    pub(super) fn apply_incoming_reaction_to_chat(
        &mut self,
        chat_id: &str,
        message_id: &str,
        emoji: &str,
    ) {
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        let Some(message) = thread
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        else {
            return;
        };
        apply_incoming_reaction(message, emoji);
    }
}

fn toggle_local_reaction(message: &mut ChatMessageSnapshot, emoji: &str) {
    let emoji = emoji.trim();
    if emoji.is_empty() {
        return;
    }
    if let Some(index) = message
        .reactions
        .iter()
        .position(|reaction| reaction.emoji == emoji)
    {
        let reaction = &mut message.reactions[index];
        if reaction.reacted_by_me {
            reaction.reacted_by_me = false;
            reaction.count = reaction.count.saturating_sub(1);
            if reaction.count == 0 {
                message.reactions.remove(index);
            }
        } else {
            reaction.reacted_by_me = true;
            reaction.count = reaction.count.saturating_add(1);
        }
    } else {
        message.reactions.push(MessageReactionSnapshot {
            emoji: emoji.to_string(),
            count: 1,
            reacted_by_me: true,
        });
    }
    sort_message_reactions(&mut message.reactions);
}

fn apply_incoming_reaction(message: &mut ChatMessageSnapshot, emoji: &str) -> bool {
    let emoji = emoji.trim();
    if emoji.is_empty() {
        return false;
    }
    if let Some(reaction) = message
        .reactions
        .iter_mut()
        .find(|reaction| reaction.emoji == emoji)
    {
        reaction.count = reaction.count.saturating_add(1);
    } else {
        message.reactions.push(MessageReactionSnapshot {
            emoji: emoji.to_string(),
            count: 1,
            reacted_by_me: false,
        });
    }
    sort_message_reactions(&mut message.reactions);
    true
}

fn sort_message_reactions(reactions: &mut [MessageReactionSnapshot]) {
    reactions.sort_by(|left, right| {
        right
            .reacted_by_me
            .cmp(&left.reacted_by_me)
            .then_with(|| right.count.cmp(&left.count))
            .then_with(|| left.emoji.cmp(&right.emoji))
    });
}
