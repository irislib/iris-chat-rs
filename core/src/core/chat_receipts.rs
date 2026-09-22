use super::*;

const DELIVERED_RECEIPT_DEBOUNCE: Duration = Duration::from_millis(750);

impl AppCore {
    pub(super) fn set_chat_unread(&mut self, chat_id: &str, unread: bool) {
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(thread) = self.threads.get(&normalized_chat_id) else {
            return;
        };
        if unread {
            let thread = self
                .threads
                .get_mut(&normalized_chat_id)
                .expect("existing thread");
            thread.unread_count = thread.unread_count.max(1);
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
            return;
        }
        let message_ids = thread
            .messages
            .iter()
            .filter(|message| !message.is_outgoing)
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        if !self.record_local_chat_read_state(&normalized_chat_id, &message_ids, true) {
            return;
        }
        self.send_seen_updates(&normalized_chat_id, message_ids);
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
        self.broadcast_device_sync_snapshot();
    }

    pub(super) fn mark_messages_seen(&mut self, chat_id: &str, message_ids: &[String]) {
        if message_ids.is_empty() {
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let Some(thread) = self.threads.get(&normalized_chat_id) else {
            return;
        };
        let receipt_ids = thread
            .messages
            .iter()
            .filter(|message| !message.is_outgoing && message_ids.contains(&message.id))
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        if receipt_ids.is_empty()
            || !self.record_local_chat_read_state(&normalized_chat_id, &receipt_ids, false)
        {
            return;
        }
        self.send_seen_updates(&normalized_chat_id, receipt_ids);
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
        self.broadcast_device_sync_snapshot();
    }

    pub(super) fn sync_open_chat_read_state(&mut self, chat_id: &str) {
        let Some(thread) = self.threads.get(chat_id) else {
            return;
        };
        let ids = thread
            .messages
            .iter()
            .filter(|message| !message.is_outgoing)
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        if self.record_local_chat_read_state(chat_id, &ids, true) {
            self.send_seen_updates(chat_id, ids);
            self.broadcast_device_sync_snapshot();
        }
    }

    fn send_seen_updates(&mut self, chat_id: &str, mut receipt_ids: Vec<String>) {
        if receipt_ids.is_empty() {
            if let Some(state) = self.chat_read_states.get(chat_id) {
                receipt_ids.extend(state.seen_at_boundary.iter().cloned());
            }
        }
        if receipt_ids.is_empty() {
            return;
        }
        self.cancel_pending_delivered_receipts(chat_id, &receipt_ids);
        self.send_seen_receipt_to_local_siblings(chat_id, receipt_ids.clone());
        if self.preferences.send_read_receipts && !self.thread_is_message_request(chat_id) {
            self.send_receipt(chat_id, "seen", receipt_ids);
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
        let Some(mut unsigned) =
            receipt_unsigned_event(owner_pubkey, "seen", message_ids, group_id.as_deref())
        else {
            return;
        };
        if let Some(state) = self.chat_read_states.get(chat_id) {
            if let Ok(encoded) = serde_json::to_string(state) {
                if let Ok(tag) = nostr::Tag::parse(["iris-read-state", &encoded]) {
                    unsigned.tags.push(tag);
                    unsigned.id = None;
                    unsigned.ensure_id();
                }
            }
        }
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

    pub(super) fn queue_delivered_receipt(&mut self, chat_id: &str, message_id: String) {
        if message_id.is_empty() {
            return;
        }
        let due_at = Instant::now() + DELIVERED_RECEIPT_DEBOUNCE;
        self.pending_delivered_receipts
            .entry((chat_id.to_string(), message_id))
            .or_insert(due_at);
        self.schedule_pending_delivered_receipt_flush();
    }

    fn schedule_pending_delivered_receipt_flush(&mut self) {
        let Some(due_at) = self.pending_delivered_receipts.values().min().copied() else {
            self.pending_delivered_receipt_flush_due_at = None;
            return;
        };
        if self
            .pending_delivered_receipt_flush_due_at
            .is_some_and(|existing| existing <= due_at)
        {
            return;
        }
        self.pending_delivered_receipt_flush_due_at = Some(due_at);
        self.pending_delivered_receipt_token = self.pending_delivered_receipt_token.wrapping_add(1);
        let token = self.pending_delivered_receipt_token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep_until(due_at).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::FlushPendingDeliveredReceipts { token },
            )));
        });
    }

    pub(super) fn handle_pending_delivered_receipt_flush(&mut self, token: u64) {
        if token != self.pending_delivered_receipt_token {
            return;
        }
        self.pending_delivered_receipt_flush_due_at = None;
        self.flush_due_pending_delivered_receipts(Instant::now());
        self.schedule_pending_delivered_receipt_flush();
    }

    fn flush_due_pending_delivered_receipts(&mut self, now: Instant) {
        let pending = std::mem::take(&mut self.pending_delivered_receipts);
        let mut due = Vec::new();
        for ((chat_id, message_id), due_at) in pending {
            if due_at <= now {
                due.push((chat_id, message_id));
            } else {
                self.pending_delivered_receipts
                    .insert((chat_id, message_id), due_at);
            }
        }
        self.send_pending_delivered_receipts(due);
    }

    #[cfg(test)]
    pub(super) fn flush_all_pending_delivered_receipts_for_test(&mut self) {
        let pending = std::mem::take(&mut self.pending_delivered_receipts);
        self.pending_delivered_receipt_flush_due_at = None;
        self.pending_delivered_receipt_token = self.pending_delivered_receipt_token.wrapping_add(1);
        self.send_pending_delivered_receipts(pending.into_keys());
    }

    fn cancel_pending_delivered_receipts(&mut self, chat_id: &str, message_ids: &[String]) {
        if message_ids.is_empty() {
            return;
        }
        let ids = message_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut removed = false;
        self.pending_delivered_receipts
            .retain(|(pending_chat_id, pending_message_id), _| {
                let keep = pending_chat_id != chat_id || !ids.contains(pending_message_id.as_str());
                removed |= !keep;
                keep
            });
        let delivered_key = (chat_id.to_string(), "delivered".to_string());
        let mut remove_batch_key = false;
        if let Some(batch_ids) = self.pending_outgoing_receipts.get_mut(&delivered_key) {
            batch_ids.retain(|id| !ids.contains(id.as_str()));
            remove_batch_key = batch_ids.is_empty();
        }
        if remove_batch_key {
            self.pending_outgoing_receipts.remove(&delivered_key);
        }
        if removed && self.pending_delivered_receipts.is_empty() {
            self.pending_delivered_receipt_flush_due_at = None;
            self.pending_delivered_receipt_token =
                self.pending_delivered_receipt_token.wrapping_add(1);
        }
    }

    fn send_pending_delivered_receipts(
        &mut self,
        pending: impl IntoIterator<Item = (String, String)>,
    ) {
        let mut by_chat = BTreeMap::<String, Vec<String>>::new();
        for (chat_id, message_id) in pending {
            if self.should_send_pending_delivered_receipt(&chat_id, &message_id) {
                by_chat.entry(chat_id).or_default().push(message_id);
            }
        }
        for (chat_id, mut ids) in by_chat {
            ids.sort();
            ids.dedup();
            if !ids.is_empty() {
                self.send_receipt(&chat_id, "delivered", ids);
            }
        }
    }

    fn should_send_pending_delivered_receipt(&self, chat_id: &str, message_id: &str) -> bool {
        if !self.preferences.send_read_receipts || self.thread_is_message_request(chat_id) {
            return false;
        }
        let Some(thread) = self.threads.get(chat_id) else {
            return false;
        };
        thread.messages.iter().any(|message| {
            !message.is_outgoing
                && message.id == message_id
                && !matches!(message.delivery, DeliveryState::Seen)
        })
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
        let unread_ids = thread.unread_message_ids();
        let mut changed = false;
        let mut newly_seen_incoming = 0u64;
        for message in &mut thread.messages {
            if !message_ids.iter().any(|id| id == &message.id) {
                continue;
            }
            if is_from_local_owner == message.is_outgoing {
                continue;
            }
            if should_advance_delivery(&message.delivery, &delivery) {
                if !message.is_outgoing && matches!(delivery, DeliveryState::Seen) {
                    newly_seen_incoming += u64::from(unread_ids.contains(&message.id));
                }
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
            // An older receipt must not clear newer, unread messages.
            let next_unread = thread.unread_count.saturating_sub(newly_seen_incoming);
            changed |= thread.unread_count != next_unread;
            thread.unread_count = next_unread;
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
