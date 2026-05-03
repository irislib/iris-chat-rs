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
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
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
        let outgoing_emoji = toggle_local_reaction(message, &local_owner, emoji);
        self.send_reaction(&normalized_chat_id, message_id, &outgoing_emoji);
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn send_reaction(&mut self, chat_id: &str, message_id: &str, emoji: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if parse_group_id_from_chat_id(chat_id).is_some() {
            self.push_debug_log(
                "group.reaction.skipped",
                "group reactions are deferred on the experimental group protocol",
            );
            return;
        }

        if let Ok((_, peer)) = parse_peer_input(chat_id) {
            if emoji.is_empty() {
                // The shared NDR helper rejects empty content, so build the
                // raw kind-7 rumor ourselves to broadcast an unreact.
                if let Ok(e_tag) = nostr::Tag::parse(["e", message_id]) {
                    let unsigned = UnsignedEvent::new(
                        peer,
                        Timestamp::from_secs(unix_now().get()),
                        Kind::Custom(REACTION_KIND as u16),
                        vec![e_tag],
                        String::new(),
                    );
                    let _ = logged_in.ndr_runtime.send_event(peer, unsigned);
                }
            } else {
                let _ = logged_in.ndr_runtime.send_reaction(
                    peer,
                    message_id.to_string(),
                    emoji.to_string(),
                    None,
                );
            }
            self.process_runtime_events();
        }
    }

    pub(super) fn apply_incoming_reaction_to_chat(
        &mut self,
        chat_id: &str,
        message_id: &str,
        sender_hex: &str,
        emoji: &str,
    ) {
        let local_owner = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex());
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
        apply_reaction_from(message, sender_hex, emoji, local_owner.as_deref());
    }
}

/// Set the local user's reaction to `emoji`, replacing any prior reaction
/// they had on this message. Picking the same emoji again toggles it off.
///
/// Returns the emoji that should be broadcast: the new emoji on react,
/// or empty string for an unreact (so peers can clear the prior choice).
fn toggle_local_reaction(
    message: &mut ChatMessageSnapshot,
    local_owner: &str,
    emoji: &str,
) -> String {
    let emoji = emoji.trim();
    if emoji.is_empty() {
        // Treat as explicit unreact.
        apply_reaction_from(message, local_owner, "", Some(local_owner));
        return String::new();
    }

    let already_picked = message
        .reactors
        .iter()
        .any(|reactor| reactor.author == local_owner && reactor.emoji == emoji);

    if already_picked {
        apply_reaction_from(message, local_owner, "", Some(local_owner));
        String::new()
    } else {
        apply_reaction_from(message, local_owner, emoji, Some(local_owner));
        emoji.to_string()
    }
}

/// Record that `sender` now has `emoji` (or no reaction, when empty) on this
/// message. One reaction per sender — a new emoji replaces the old one.
fn apply_reaction_from(
    message: &mut ChatMessageSnapshot,
    sender: &str,
    emoji: &str,
    local_owner: Option<&str>,
) {
    if sender.is_empty() {
        return;
    }
    let emoji = emoji.trim().to_string();

    if let Some(index) = message
        .reactors
        .iter()
        .position(|reactor| reactor.author == sender)
    {
        if emoji.is_empty() {
            message.reactors.remove(index);
        } else {
            message.reactors[index].emoji = emoji;
        }
    } else if !emoji.is_empty() {
        message.reactors.push(MessageReactor {
            author: sender.to_string(),
            emoji,
        });
    }

    rebuild_reaction_aggregate(message, local_owner);
}

/// Recompute `reactions` from `reactors`, sorted with the local user's
/// reaction first, then by descending count, then alphabetically.
fn rebuild_reaction_aggregate(message: &mut ChatMessageSnapshot, local_owner: Option<&str>) {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, (u64, bool)> = BTreeMap::new();
    for reactor in &message.reactors {
        if reactor.emoji.is_empty() {
            continue;
        }
        let entry = counts.entry(reactor.emoji.clone()).or_insert((0, false));
        entry.0 = entry.0.saturating_add(1);
        if local_owner.is_some_and(|me| me == reactor.author) {
            entry.1 = true;
        }
    }
    let mut reactions: Vec<MessageReactionSnapshot> = counts
        .into_iter()
        .map(|(emoji, (count, reacted_by_me))| MessageReactionSnapshot {
            emoji,
            count,
            reacted_by_me,
        })
        .collect();
    reactions.sort_by(|left, right| {
        right
            .reacted_by_me
            .cmp(&left.reacted_by_me)
            .then_with(|| right.count.cmp(&left.count))
            .then_with(|| left.emoji.cmp(&right.emoji))
    });
    message.reactions = reactions;
}
