use super::*;

impl AppCore {
    pub(super) fn mark_messages_seen(&mut self, chat_id: &str, message_ids: &[String]) {
        if message_ids.is_empty() {
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(thread) = self.threads.get_mut(&normalized_chat_id) else {
            return;
        };

        let mut changed = false;
        let mut receipt_ids = Vec::new();
        for message in &mut thread.messages {
            if message.is_outgoing || !message_ids.iter().any(|id| id == &message.id) {
                continue;
            }
            if should_advance_delivery(&message.delivery, &DeliveryState::Seen) {
                message.delivery = DeliveryState::Seen;
                changed = true;
            }
            receipt_ids.push(message.id.clone());
        }
        if receipt_ids.is_empty() {
            return;
        }

        if thread.unread_count != 0 {
            thread.unread_count = 0;
            changed = true;
        }
        if self.preferences.send_read_receipts {
            self.send_receipt(&normalized_chat_id, "seen", receipt_ids);
        }

        if changed {
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
        }
    }

    pub(super) fn send_receipt(
        &mut self,
        chat_id: &str,
        receipt_type: &str,
        message_ids: Vec<String>,
    ) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if is_group_chat_id(chat_id) {
            let tags = message_ids
                .into_iter()
                .map(|id| vec!["e".to_string(), id])
                .collect();
            self.send_group_event(chat_id, RECEIPT_KIND, receipt_type, tags, None);
        } else if let Ok((_, peer)) = parse_peer_input(chat_id) {
            if let Ok(result) =
                logged_in
                    .ndr_runtime
                    .send_receipt(peer, receipt_type, message_ids, None)
            {
                self.process_runtime_effects(result.effects);
            }
        }
    }

    pub(super) fn apply_receipt_to_messages(
        &mut self,
        chat_id: &str,
        message_ids: &[String],
        delivery: DeliveryState,
        is_from_local_owner: bool,
        receipt_author_hex: Option<&str>,
    ) {
        if message_ids.is_empty() {
            return;
        }
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        let mut changed = false;
        for message in &mut thread.messages {
            if !message_ids.iter().any(|id| id == &message.id) {
                continue;
            }
            if is_from_local_owner == message.is_outgoing {
                continue;
            }
            if should_advance_delivery(&message.delivery, &delivery) {
                message.delivery = delivery.clone();
                changed = true;
            }
            if message.is_outgoing {
                if let Some(author) = receipt_author_hex {
                    let now = unix_now().get();
                    if let Some(recipient) = message
                        .recipient_deliveries
                        .iter_mut()
                        .find(|recipient| recipient.owner_pubkey_hex == author)
                    {
                        if should_advance_delivery(&recipient.delivery, &delivery) {
                            recipient.delivery = delivery.clone();
                            recipient.updated_at_secs = now;
                            changed = true;
                        }
                    } else {
                        message
                            .recipient_deliveries
                            .push(MessageRecipientDeliverySnapshot {
                                owner_pubkey_hex: author.to_string(),
                                delivery: delivery.clone(),
                                updated_at_secs: now,
                            });
                        changed = true;
                    }
                }
            }
        }
        if is_from_local_owner && matches!(delivery, DeliveryState::Seen) {
            thread.unread_count = 0;
            changed = true;
        }
        if changed {
            self.persist_best_effort();
        }
    }
}

fn should_advance_delivery(current: &DeliveryState, next: &DeliveryState) -> bool {
    delivery_rank(next) > delivery_rank(current)
}

fn delivery_rank(state: &DeliveryState) -> u8 {
    match state {
        DeliveryState::Queued => 0,
        DeliveryState::Pending => 1,
        DeliveryState::Sent => 2,
        DeliveryState::Received => 3,
        DeliveryState::Seen => 4,
        DeliveryState::Failed => 0,
    }
}
