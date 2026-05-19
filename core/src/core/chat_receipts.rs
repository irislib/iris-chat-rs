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
        self.send_seen_receipt_to_local_siblings(&normalized_chat_id, receipt_ids.clone());
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

    fn send_seen_receipt_to_local_siblings(&mut self, chat_id: &str, message_ids: Vec<String>) {
        let Some(owner_pubkey) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let (conversation_owner, group_id) = if is_group_chat_id(chat_id) {
            (owner_pubkey, parse_group_id_from_chat_id(chat_id))
        } else if let Ok((_, peer)) = parse_peer_input(chat_id) {
            (peer, None)
        } else {
            return;
        };
        let Some(unsigned) =
            receipt_unsigned_event(owner_pubkey, "seen", message_ids, group_id.as_deref())
        else {
            return;
        };
        self.send_protocol_engine_unsigned_event_to_local_siblings(
            conversation_owner,
            chat_id,
            unsigned,
            "receipt.self_sync",
        );
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
            self.send_group_author_receipts(owner_pubkey, chat_id, receipt_type, message_ids);
        } else if let Ok((_, peer)) = parse_peer_input(chat_id) {
            self.send_pairwise_receipt(
                owner_pubkey,
                peer,
                chat_id,
                receipt_type,
                message_ids,
                None,
            );
        }
    }

    fn send_group_author_receipts(
        &mut self,
        owner_pubkey: PublicKey,
        chat_id: &str,
        receipt_type: &str,
        message_ids: Vec<String>,
    ) {
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            return;
        };
        let local_owner_hex = owner_pubkey.to_hex();
        for (author_hex, author_message_ids) in
            self.group_receipt_message_ids_by_author(chat_id, message_ids)
        {
            if author_hex == local_owner_hex {
                continue;
            }
            let Ok(author) = PublicKey::parse(&author_hex) else {
                continue;
            };
            self.send_pairwise_receipt(
                owner_pubkey,
                author,
                chat_id,
                receipt_type,
                author_message_ids,
                Some(&group_id),
            );
        }
    }

    fn group_receipt_message_ids_by_author(
        &self,
        chat_id: &str,
        message_ids: Vec<String>,
    ) -> BTreeMap<String, Vec<String>> {
        let requested = message_ids.into_iter().collect::<HashSet<_>>();
        let mut by_author = BTreeMap::<String, Vec<String>>::new();
        let Some(thread) = self.threads.get(chat_id) else {
            return by_author;
        };
        for message in &thread.messages {
            if message.is_outgoing || !requested.contains(&message.id) {
                continue;
            }
            let Some(author_hex) = group_message_author_hex(message) else {
                continue;
            };
            by_author
                .entry(author_hex.to_string())
                .or_default()
                .push(message.id.clone());
        }
        by_author
    }

    fn send_pairwise_receipt(
        &mut self,
        owner_pubkey: PublicKey,
        peer: PublicKey,
        chat_id: &str,
        receipt_type: &str,
        message_ids: Vec<String>,
        group_id: Option<&str>,
    ) {
        if message_ids.is_empty() {
            return;
        }
        let Some(unsigned) =
            receipt_unsigned_event(owner_pubkey, receipt_type, message_ids, group_id)
        else {
            return;
        };
        self.send_protocol_engine_unsigned_event_to_peer_only(peer, chat_id, unsigned, "receipt");
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

fn receipt_unsigned_event(
    owner_pubkey: PublicKey,
    receipt_type: &str,
    message_ids: Vec<String>,
    group_id: Option<&str>,
) -> Option<UnsignedEvent> {
    let now = unix_now();
    let receipt_type_for_pairwise = match receipt_type {
        "seen" => pairwise_codec::ReceiptType::Seen,
        _ => pairwise_codec::ReceiptType::Delivered,
    };
    let mut unsigned = pairwise_codec::receipt_event(
        owner_pubkey,
        receipt_type_for_pairwise,
        message_ids,
        pairwise_codec::EncodeOptions::new(now.get(), now.get().saturating_mul(1000)),
    )
    .ok()?;
    if let Some(group_id) = group_id {
        if let Ok(group_tag) = nostr::Tag::parse(["l", group_id]) {
            unsigned.tags.push(group_tag);
            unsigned.id = None;
            unsigned.ensure_id();
        }
    }
    Some(unsigned)
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

fn group_message_author_hex(message: &ChatMessageSnapshot) -> Option<&str> {
    message.author_owner_pubkey_hex.as_deref().or_else(|| {
        PublicKey::parse(&message.author)
            .ok()
            .map(|_| message.author.as_str())
    })
}
