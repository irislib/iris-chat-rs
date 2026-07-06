use super::*;

impl AppCore {
    pub(in crate::core) fn drain_queued_direct_text_messages(
        &mut self,
        reason: &'static str,
    ) -> bool {
        let candidates = self
            .threads
            .iter()
            .flat_map(|(chat_id, thread)| {
                thread
                    .messages
                    .iter()
                    .filter(|message| is_queued_direct_text_message(chat_id, message))
                    .map(|message| (chat_id.clone(), message.clone()))
            })
            .collect::<Vec<_>>();

        let mut changed = false;
        let mut blocked = false;
        for (chat_id, message) in candidates {
            let Ok((normalized_chat_id, peer_pubkey)) = parse_peer_input(&chat_id) else {
                continue;
            };
            let drained = self.drain_queued_direct_text_message(
                &normalized_chat_id,
                peer_pubkey,
                &message,
                reason,
            );
            changed |= drained.changed;
            blocked |= drained.blocked;
        }
        if changed || blocked {
            self.request_protocol_subscription_refresh();
        }
        changed
    }

    fn drain_queued_direct_text_message(
        &mut self,
        chat_id: &str,
        peer_pubkey: PublicKey,
        message: &ChatMessageSnapshot,
        reason: &'static str,
    ) -> DirectTextDrainResult {
        let readiness = self
            .protocol_engine
            .as_ref()
            .map(|engine| engine.direct_send_readiness(peer_pubkey))
            .unwrap_or(DirectSendReadiness::MissingLocalAppKeys);
        if !readiness.is_ready() {
            self.push_debug_log(
                "message.direct.queue.wait",
                format!(
                    "reason={reason} chat_id={chat_id} message_id={} readiness={readiness:?}",
                    message.id
                ),
            );
            return DirectTextDrainResult {
                changed: false,
                blocked: true,
            };
        }

        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return DirectTextDrainResult {
                changed: false,
                blocked: true,
            };
        };
        let result = protocol_engine.send_direct_text(
            peer_pubkey,
            chat_id,
            &message.body,
            message.expires_at_secs,
            unix_now(),
        );
        match result {
            Ok(result) if !result.event_ids.is_empty() => {
                self.replace_queued_direct_text_message(
                    chat_id,
                    message,
                    result.message_id.clone(),
                );
                self.push_debug_log(
                    "message.direct.queue.drain",
                    format!(
                        "reason={reason} chat_id={chat_id} old_message_id={} new_message_id={} event_ids={}",
                        message.id,
                        result.message_id,
                        result.event_ids.len()
                    ),
                );
                self.process_protocol_engine_effects(result.effects);
                self.sync_message_delivery_trace(chat_id, &result.message_id);
                self.reconcile_outgoing_message_delivery(chat_id, &result.message_id);
                DirectTextDrainResult {
                    changed: true,
                    blocked: false,
                }
            }
            Ok(result) => {
                self.push_debug_log(
                    "message.direct.queue.invariant",
                    format!(
                        "reason={reason} chat_id={chat_id} old_message_id={} new_message_id={} event_ids=0",
                        message.id, result.message_id
                    ),
                );
                DirectTextDrainResult {
                    changed: false,
                    blocked: false,
                }
            }
            Err(error) => {
                self.push_debug_log(
                    "message.direct.queue.error",
                    format!(
                        "reason={reason} chat_id={chat_id} message_id={} error={error}",
                        message.id
                    ),
                );
                DirectTextDrainResult {
                    changed: false,
                    blocked: false,
                }
            }
        }
    }

    fn replace_queued_direct_text_message(
        &mut self,
        chat_id: &str,
        queued_message: &ChatMessageSnapshot,
        final_message_id: String,
    ) {
        if let Some(thread) = self.threads.get_mut(chat_id) {
            thread
                .messages
                .retain(|message| message.id != queued_message.id);
        }
        if final_message_id != queued_message.id {
            if let Err(error) = self.app_store.delete_message(chat_id, &queued_message.id) {
                self.push_debug_log(
                    "storage.message.delete.error",
                    format!(
                        "chat_id={chat_id} message_id={} error={error}",
                        queued_message.id
                    ),
                );
            }
        }
        self.push_outgoing_message_with_id(
            final_message_id,
            chat_id,
            queued_message.body.clone(),
            queued_message.created_at_secs,
            queued_message.expires_at_secs,
            DeliveryState::Pending,
        );
    }
}

struct DirectTextDrainResult {
    changed: bool,
    blocked: bool,
}

pub(super) fn is_queued_direct_text_message(chat_id: &str, message: &ChatMessageSnapshot) -> bool {
    !is_group_chat_id(chat_id)
        && message.is_outgoing
        && matches!(message.kind, ChatMessageKind::User)
        && matches!(message.delivery, DeliveryState::Queued)
        && message.delivery_trace.outer_event_ids.is_empty()
        && message.delivery_trace.pending_relay_event_ids.is_empty()
        && message.source_event_id.is_none()
}
