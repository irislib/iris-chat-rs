use super::*;

impl AppCore {
    pub(super) fn set_chat_unread(&mut self, chat_id: &str, unread: bool) {
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(thread) = self.threads.get_mut(&normalized_chat_id) else {
            return;
        };

        let next_unread = if unread {
            thread.unread_count.max(1)
        } else {
            0
        };
        if thread.unread_count == next_unread {
            return;
        }
        thread.unread_count = next_unread;
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

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
        if self.preferences.send_read_receipts
            && !self.thread_is_message_request(&normalized_chat_id)
        {
            // Don't emit seen receipts for an unaccepted message
            // request — the sender shouldn't get "read" feedback
            // until the recipient has opted in by tapping Accept.
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
        if message_ids.is_empty() {
            return;
        }
        // Within a batch (catch-up flurry, multi-action handling, …), queue
        // ids by (chat, receipt_type) and let exit_batch flush them all at
        // once. Outside a batch — e.g. a single `markMessagesSeen` from the
        // shell — fall through and send immediately, same as before.
        if self.batch_depth > 0 {
            self.pending_outgoing_receipts
                .entry((chat_id.to_string(), receipt_type.to_string()))
                .or_default()
                .extend(message_ids);
            return;
        }
        self.send_receipt_inner(chat_id, receipt_type, message_ids);
    }

    pub(super) fn send_receipt_inner(
        &mut self,
        chat_id: &str,
        receipt_type: &str,
        message_ids: Vec<String>,
    ) {
        let Some(owner_pubkey) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        if is_group_chat_id(chat_id) {
            let tags = message_ids
                .into_iter()
                .map(|id| vec!["e".to_string(), id])
                .collect();
            self.send_group_event(chat_id, RECEIPT_KIND, receipt_type, tags, None);
        } else if let Ok((_, peer)) = parse_peer_input(chat_id) {
            let now = unix_now();
            let receipt_type_for_pairwise = match receipt_type {
                "seen" => pairwise_codec::ReceiptType::Seen,
                _ => pairwise_codec::ReceiptType::Delivered,
            };
            if let Ok(unsigned) = pairwise_codec::receipt_event(
                owner_pubkey,
                receipt_type_for_pairwise,
                message_ids.clone(),
                pairwise_codec::EncodeOptions::new(now.get(), now.get().saturating_mul(1000)),
            ) {
                self.send_protocol_engine_unsigned_event(peer, chat_id, unsigned, "receipt");
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
                                display_name: String::new(),
                                picture_url: None,
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
