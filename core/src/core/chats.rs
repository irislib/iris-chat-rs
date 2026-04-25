use super::*;

const TYPING_INDICATOR_TTL_SECS: u64 = 10;

impl AppCore {
    pub(super) fn create_chat(&mut self, peer_input: &str) {
        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        }
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        self.state.busy.creating_chat = true;
        self.emit_state();

        let Ok((chat_id, _pubkey)) = parse_peer_input(peer_input) else {
            self.state.toast = Some("Invalid peer key.".to_string());
            self.state.busy.creating_chat = false;
            self.emit_state();
            return;
        };
        self.push_debug_log(
            "chat.create",
            format!("peer_input={} chat_id={chat_id}", peer_input.trim()),
        );

        let now = unix_now().get();
        self.prune_recent_handshake_peers(now);
        self.ensure_thread_record(&chat_id, now).unread_count = 0;

        self.active_chat_id = Some(chat_id.clone());
        self.screen_stack = vec![Screen::Chat { chat_id }];
        self.republish_local_identity_artifacts();
        self.rebuild_state();
        self.persist_best_effort();
        self.request_protocol_subscription_refresh();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
        self.state.busy.creating_chat = false;
        self.emit_state();
    }

    pub(super) fn ensure_thread_record(
        &mut self,
        chat_id: &str,
        updated_at_secs: u64,
    ) -> &mut ThreadRecord {
        let thread = self
            .threads
            .entry(chat_id.to_string())
            .or_insert_with(|| ThreadRecord {
                chat_id: chat_id.to_string(),
                unread_count: 0,
                updated_at_secs,
                messages: Vec::new(),
            });
        if thread.updated_at_secs == 0 {
            thread.updated_at_secs = updated_at_secs;
        }
        thread
    }

    pub(super) fn normalize_chat_id(&self, chat_id: &str) -> Option<String> {
        if is_group_chat_id(chat_id) {
            let group_id = parse_group_id_from_chat_id(chat_id)?;
            let group_chat_id = group_chat_id(&group_id);
            let known_group = self
                .logged_in
                .as_ref()
                .and_then(|logged_in| logged_in.group_manager.group(&group_id))
                .is_some();
            if known_group || self.threads.contains_key(&group_chat_id) {
                return Some(group_chat_id);
            }
            return None;
        }

        parse_peer_input(chat_id)
            .ok()
            .map(|(normalized, _)| normalized)
    }

    pub(super) fn open_chat(&mut self, chat_id: &str) {
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        let Some(chat_id) = self.normalize_chat_id(chat_id) else {
            self.state.toast = Some("Invalid chat id.".to_string());
            self.emit_state();
            return;
        };

        let now = unix_now().get();
        self.prune_recent_handshake_peers(now);
        self.ensure_thread_record(&chat_id, now).unread_count = 0;
        self.active_chat_id = Some(chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }];
        self.republish_local_identity_artifacts();
        self.rebuild_state();
        self.persist_best_effort();
        self.request_protocol_subscription_refresh();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
        self.emit_state();
    }

    pub(super) fn send_message(&mut self, chat_id: &str, text: &str, expires_at_secs: Option<u64>) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        }
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            self.state.toast = Some("Invalid chat id.".to_string());
            self.emit_state();
            return;
        };
        self.push_debug_log(
            "chat.send",
            format!(
                "chat_id={} is_group={}",
                normalized_chat_id,
                is_group_chat_id(&normalized_chat_id)
            ),
        );

        let now = unix_now();
        self.prune_recent_handshake_peers(now.get());
        self.active_chat_id = Some(normalized_chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: normalized_chat_id.clone(),
        }];
        self.ensure_thread_record(&normalized_chat_id, now.get());
        let expires_at_secs = expires_at_secs.or_else(|| {
            self.chat_message_ttl_seconds
                .get(&normalized_chat_id)
                .map(|ttl_seconds| now.get().saturating_add(*ttl_seconds))
        });
        self.state.busy.sending_message = true;
        self.rebuild_state();
        self.emit_state();

        if is_group_chat_id(&normalized_chat_id) {
            self.send_group_message(&normalized_chat_id, trimmed, now, expires_at_secs);
        } else {
            self.send_direct_message(&normalized_chat_id, trimmed, now, expires_at_secs);
        }

        self.schedule_next_pending_retry(now.get());
        self.state.busy.sending_message = false;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn send_direct_message(
        &mut self,
        chat_id: &str,
        text: &str,
        now: UnixSeconds,
        expires_at_secs: Option<u64>,
    ) {
        let Ok((normalized_chat_id, peer_pubkey)) = parse_peer_input(chat_id) else {
            self.state.toast = Some("Invalid peer key.".to_string());
            return;
        };

        let message_id = self.allocate_message_id();
        let payload =
            match encode_app_direct_message_payload(&normalized_chat_id, &message_id, text) {
                Ok(payload) => payload,
                Err(error) => {
                    self.state.toast = Some(error.to_string());
                    return;
                }
            };
        let owner = OwnerPubkey::from_bytes(peer_pubkey.to_bytes());
        let prepared = {
            let logged_in = self.logged_in.as_mut().expect("logged in checked above");
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            logged_in
                .session_manager
                .prepare_send(&mut ctx, owner, payload)
        };

        self.handle_prepared_direct_send(
            &normalized_chat_id,
            message_id,
            text,
            now,
            expires_at_secs,
            prepared,
        );
    }

    pub(super) fn send_group_message(
        &mut self,
        chat_id: &str,
        text: &str,
        now: UnixSeconds,
        expires_at_secs: Option<u64>,
    ) {
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            self.state.toast = Some("Invalid group id.".to_string());
            return;
        };
        let message_id = self.allocate_message_id();
        let payload = match encode_app_group_message_payload(&message_id, text) {
            Ok(payload) => payload,
            Err(error) => {
                self.state.toast = Some(error.to_string());
                return;
            }
        };

        let prepared = {
            let logged_in = self.logged_in.as_mut().expect("logged in checked above");
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            let (session_manager, group_manager) =
                (&mut logged_in.session_manager, &mut logged_in.group_manager);
            group_manager.send_message(session_manager, &mut ctx, &group_id, payload)
        };

        match prepared {
            Ok(prepared) => {
                self.publish_group_local_sibling_best_effort(&prepared);
                if let Some(reason) = pending_reason_from_group_prepared(&prepared) {
                    self.push_debug_log(
                        "group.send.pending",
                        format!(
                            "chat_id={} reason={reason:?} gaps={}",
                            chat_id,
                            summarize_relay_gaps(&prepared.remote.relay_gaps)
                        ),
                    );
                    let pending_reason = reason.clone();
                    let message = self.push_outgoing_message_with_id(
                        message_id.clone(),
                        chat_id,
                        text.to_string(),
                        now.get(),
                        expires_at_secs,
                        DeliveryState::Queued,
                    );
                    self.queue_pending_outbound(
                        message.id,
                        chat_id.to_string(),
                        text.to_string(),
                        expires_at_secs,
                        None,
                        OutboundPublishMode::WaitForPeer,
                        pending_reason.clone(),
                        now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                    );
                    self.nudge_protocol_state_for_pending_reason(&pending_reason);
                    self.request_protocol_subscription_refresh();
                    self.schedule_pending_outbound_retry(Duration::from_secs(
                        PENDING_RETRY_DELAY_SECS,
                    ));
                } else {
                    match build_group_prepared_publish_batch(&prepared, expires_at_secs) {
                        Ok(Some(batch)) => {
                            let publish_mode = publish_mode_for_batch(&batch);
                            let message = self.push_outgoing_message_with_id(
                                message_id.clone(),
                                chat_id,
                                text.to_string(),
                                now.get(),
                                expires_at_secs,
                                DeliveryState::Pending,
                            );
                            self.queue_pending_outbound(
                                message.id.clone(),
                                chat_id.to_string(),
                                text.to_string(),
                                expires_at_secs,
                                Some(batch.clone()),
                                publish_mode.clone(),
                                pending_reason_for_publish_mode(&publish_mode),
                                retry_deadline_for_publish_mode(now.get(), &publish_mode),
                            );
                            self.set_pending_outbound_in_flight(&message.id, true);
                            self.start_publish_for_pending(
                                message.id,
                                chat_id.to_string(),
                                publish_mode,
                                batch,
                            );
                        }
                        Ok(None) => {
                            let message = self.push_outgoing_message_with_id(
                                message_id.clone(),
                                chat_id,
                                text.to_string(),
                                now.get(),
                                expires_at_secs,
                                DeliveryState::Failed,
                            );
                            self.update_message_delivery(
                                chat_id,
                                &message.id,
                                DeliveryState::Failed,
                            );
                        }
                        Err(error) => self.state.toast = Some(error.to_string()),
                    }
                }
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
            }
        }
    }

    pub(super) fn handle_prepared_direct_send(
        &mut self,
        chat_id: &str,
        message_id: String,
        text: &str,
        now: UnixSeconds,
        expires_at_secs: Option<u64>,
        prepared: Result<nostr_double_ratchet::PreparedSend, Error>,
    ) {
        match prepared {
            Ok(prepared) => {
                if let Some(reason) = pending_reason_from_prepared(&prepared) {
                    self.push_debug_log(
                        "direct.send.pending",
                        format!(
                            "chat_id={} reason={reason:?} gaps={}",
                            chat_id,
                            summarize_relay_gaps(&prepared.relay_gaps)
                        ),
                    );
                    let pending_reason = reason.clone();
                    let message = self.push_outgoing_message_with_id(
                        message_id.clone(),
                        chat_id,
                        text.to_string(),
                        now.get(),
                        expires_at_secs,
                        DeliveryState::Queued,
                    );
                    self.queue_pending_outbound(
                        message.id,
                        chat_id.to_string(),
                        text.to_string(),
                        expires_at_secs,
                        None,
                        OutboundPublishMode::WaitForPeer,
                        pending_reason.clone(),
                        now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                    );
                    self.nudge_protocol_state_for_pending_reason(&pending_reason);
                    self.request_protocol_subscription_refresh();
                    self.schedule_pending_outbound_retry(Duration::from_secs(
                        PENDING_RETRY_DELAY_SECS,
                    ));
                } else {
                    match build_prepared_publish_batch(&prepared, expires_at_secs) {
                        Ok(Some(batch)) => {
                            let publish_mode = publish_mode_for_batch(&batch);
                            let message = self.push_outgoing_message_with_id(
                                message_id.clone(),
                                chat_id,
                                text.to_string(),
                                now.get(),
                                expires_at_secs,
                                DeliveryState::Pending,
                            );
                            self.queue_pending_outbound(
                                message.id.clone(),
                                chat_id.to_string(),
                                text.to_string(),
                                expires_at_secs,
                                Some(batch.clone()),
                                publish_mode.clone(),
                                pending_reason_for_publish_mode(&publish_mode),
                                retry_deadline_for_publish_mode(now.get(), &publish_mode),
                            );
                            self.set_pending_outbound_in_flight(&message.id, true);
                            self.start_publish_for_pending(
                                message.id,
                                chat_id.to_string(),
                                publish_mode,
                                batch,
                            );
                        }
                        Ok(None) => {
                            let message = self.push_outgoing_message_with_id(
                                message_id.clone(),
                                chat_id,
                                text.to_string(),
                                now.get(),
                                expires_at_secs,
                                DeliveryState::Failed,
                            );
                            self.update_message_delivery(
                                chat_id,
                                &message.id,
                                DeliveryState::Failed,
                            );
                        }
                        Err(error) => self.state.toast = Some(error.to_string()),
                    }
                }
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
            }
        }
    }

    pub(super) fn retry_pending_inbound(&mut self, now: UnixSeconds) {
        if self.logged_in.is_none() {
            return;
        }

        let mut pending = std::mem::take(&mut self.pending_inbound);
        loop {
            let mut still_pending = Vec::new();
            let mut made_progress = false;

            for item in pending {
                if let PendingInbound::Decrypted {
                    sender_owner_hex,
                    payload,
                    created_at_secs,
                    expires_at_secs,
                } = item.clone()
                {
                    let Ok(sender_pubkey) = PublicKey::parse(&sender_owner_hex) else {
                        still_pending.push(item);
                        continue;
                    };
                    match self.apply_decrypted_payload(
                        OwnerPubkey::from_bytes(sender_pubkey.to_bytes()),
                        &payload,
                        created_at_secs,
                        expires_at_secs,
                    ) {
                        Ok(()) => {
                            made_progress = true;
                        }
                        Err(error) if is_retryable_group_payload_error(&error) => {
                            if is_unknown_group_payload_error(&error) {
                                self.fetch_recent_messages_for_owner_with_lookback(
                                    OwnerPubkey::from_bytes(sender_pubkey.to_bytes()),
                                    now,
                                    UNKNOWN_GROUP_RECOVERY_LOOKBACK_SECS,
                                );
                            }
                            still_pending.push(item);
                        }
                        Err(error) => {
                            self.state.toast = Some(error.to_string());
                            made_progress = true;
                        }
                    }
                    continue;
                }

                let PendingInbound::Envelope {
                    envelope,
                    expires_at_secs,
                } = &item
                else {
                    continue;
                };

                let sender_owner = self.logged_in.as_ref().and_then(|logged_in| {
                    resolve_message_sender_owner(&logged_in.session_manager, envelope, now)
                });
                let Some(sender_owner) = sender_owner else {
                    still_pending.push(item);
                    continue;
                };
                let receive_result = {
                    let logged_in = self.logged_in.as_mut().expect("checked above");
                    let mut rng = OsRng;
                    let mut ctx = ProtocolContext::new(now, &mut rng);
                    logged_in
                        .session_manager
                        .receive(&mut ctx, sender_owner, envelope)
                };
                match receive_result {
                    Ok(Some(message)) => match self.apply_decrypted_payload(
                        message.owner_pubkey,
                        &message.payload,
                        envelope.created_at.get(),
                        *expires_at_secs,
                    ) {
                        Ok(()) => {
                            made_progress = true;
                        }
                        Err(error) if is_retryable_group_payload_error(&error) => {
                            if is_unknown_group_payload_error(&error) {
                                self.fetch_recent_messages_for_owner_with_lookback(
                                    message.owner_pubkey,
                                    now,
                                    UNKNOWN_GROUP_RECOVERY_LOOKBACK_SECS,
                                );
                            }
                            still_pending.push(PendingInbound::decrypted(
                                message.owner_pubkey,
                                message.payload,
                                envelope.created_at.get(),
                                *expires_at_secs,
                            ));
                        }
                        Err(error) => {
                            self.state.toast = Some(error.to_string());
                            made_progress = true;
                        }
                    },
                    Ok(None) | Err(_) => {
                        // If the owner is now resolvable but the real session manager can no
                        // longer receive this envelope, the payload was already consumed earlier.
                        // Keeping the raw envelope would wedge the queue forever.
                        made_progress = true;
                    }
                }
            }

            if still_pending.is_empty() || !made_progress {
                self.pending_inbound = still_pending;
                break;
            }
            pending = still_pending;
        }
    }

    pub(super) fn retry_pending_outbound(&mut self, now: UnixSeconds) {
        if self.logged_in.is_none() || self.pending_outbound.is_empty() {
            return;
        }

        self.prune_recent_handshake_peers(now.get());
        let pending = std::mem::take(&mut self.pending_outbound);
        let mut still_pending = Vec::new();

        for mut pending_message in pending {
            if pending_message
                .expires_at_secs
                .is_some_and(|expires_at_secs| expires_at_secs <= now.get())
            {
                self.update_message_delivery(
                    &pending_message.chat_id,
                    &pending_message.message_id,
                    DeliveryState::Failed,
                );
                continue;
            }

            if pending_message.next_retry_at_secs > now.get() {
                still_pending.push(pending_message);
                continue;
            }

            if pending_message.in_flight {
                still_pending.push(pending_message);
                continue;
            }

            if let Some(batch) = pending_message.prepared_publish.clone() {
                pending_message.publish_mode =
                    migrate_publish_mode(pending_message.publish_mode.clone(), Some(&batch));
                pending_message.reason =
                    pending_reason_for_publish_mode(&pending_message.publish_mode);
                pending_message.next_retry_at_secs =
                    retry_deadline_for_publish_mode(now.get(), &pending_message.publish_mode);
                pending_message.in_flight = true;
                self.update_message_delivery(
                    &pending_message.chat_id,
                    &pending_message.message_id,
                    DeliveryState::Pending,
                );
                self.start_publish_for_pending(
                    pending_message.message_id.clone(),
                    pending_message.chat_id.clone(),
                    pending_message.publish_mode.clone(),
                    batch,
                );
                still_pending.push(pending_message);
                continue;
            }

            if is_group_chat_id(&pending_message.chat_id) {
                let Some(group_id) = parse_group_id_from_chat_id(&pending_message.chat_id) else {
                    self.update_message_delivery(
                        &pending_message.chat_id,
                        &pending_message.message_id,
                        DeliveryState::Failed,
                    );
                    continue;
                };
                let payload = match encode_app_group_message_payload(
                    &pending_message.message_id,
                    &pending_message.body,
                ) {
                    Ok(payload) => payload,
                    Err(error) => {
                        self.state.toast = Some(error.to_string());
                        self.update_message_delivery(
                            &pending_message.chat_id,
                            &pending_message.message_id,
                            DeliveryState::Failed,
                        );
                        continue;
                    }
                };
                let prepared = {
                    let logged_in = self.logged_in.as_mut().expect("checked above");
                    let mut rng = OsRng;
                    let mut ctx = ProtocolContext::new(now, &mut rng);
                    let (session_manager, group_manager) =
                        (&mut logged_in.session_manager, &mut logged_in.group_manager);
                    group_manager.send_message(session_manager, &mut ctx, &group_id, payload)
                };

                match prepared {
                    Ok(prepared) => {
                        self.publish_group_local_sibling_best_effort(&prepared);
                        if let Some(reason) = pending_reason_from_group_prepared(&prepared) {
                            self.push_debug_log(
                                "retry.group.pending",
                                format!(
                                    "chat_id={} reason={reason:?} gaps={}",
                                    pending_message.chat_id,
                                    summarize_relay_gaps(&prepared.remote.relay_gaps)
                                ),
                            );
                            pending_message.reason = reason.clone();
                            pending_message.next_retry_at_secs =
                                now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                            self.update_message_delivery(
                                &pending_message.chat_id,
                                &pending_message.message_id,
                                DeliveryState::Queued,
                            );
                            self.nudge_protocol_state_for_pending_reason(&reason);
                            pending_message.publish_mode = OutboundPublishMode::WaitForPeer;
                            still_pending.push(pending_message);
                        } else {
                            match build_group_prepared_publish_batch(
                                &prepared,
                                pending_message.expires_at_secs,
                            ) {
                                Ok(Some(batch)) => {
                                    pending_message.publish_mode = publish_mode_for_batch(&batch);
                                    pending_message.prepared_publish = Some(batch.clone());
                                    pending_message.reason = pending_reason_for_publish_mode(
                                        &pending_message.publish_mode,
                                    );
                                    pending_message.next_retry_at_secs =
                                        retry_deadline_for_publish_mode(
                                            now.get(),
                                            &pending_message.publish_mode,
                                        );
                                    pending_message.in_flight = true;
                                    self.update_message_delivery(
                                        &pending_message.chat_id,
                                        &pending_message.message_id,
                                        DeliveryState::Pending,
                                    );
                                    self.start_publish_for_pending(
                                        pending_message.message_id.clone(),
                                        pending_message.chat_id.clone(),
                                        pending_message.publish_mode.clone(),
                                        batch,
                                    );
                                    still_pending.push(pending_message);
                                }
                                Ok(None) => {
                                    pending_message.publish_mode = OutboundPublishMode::WaitForPeer;
                                    pending_message.reason = PendingSendReason::MissingDeviceInvite;
                                    pending_message.next_retry_at_secs =
                                        now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                                    self.update_message_delivery(
                                        &pending_message.chat_id,
                                        &pending_message.message_id,
                                        DeliveryState::Queued,
                                    );
                                    self.push_debug_log(
                                        "retry.group.pending",
                                        format!(
                                            "chat_id={} reason={:?}",
                                            pending_message.chat_id, pending_message.reason
                                        ),
                                    );
                                    self.nudge_protocol_state_for_pending_reason(
                                        &pending_message.reason,
                                    );
                                    still_pending.push(pending_message);
                                }
                                Err(error) => {
                                    self.state.toast = Some(error.to_string());
                                    self.update_message_delivery(
                                        &pending_message.chat_id,
                                        &pending_message.message_id,
                                        DeliveryState::Failed,
                                    );
                                }
                            }
                        }
                    }
                    Err(error) => {
                        self.state.toast = Some(error.to_string());
                        self.update_message_delivery(
                            &pending_message.chat_id,
                            &pending_message.message_id,
                            DeliveryState::Failed,
                        );
                    }
                }
                continue;
            }

            let prepared = {
                let owner = match parse_peer_input(&pending_message.chat_id) {
                    Ok((_, peer_pubkey)) => OwnerPubkey::from_bytes(peer_pubkey.to_bytes()),
                    Err(_) => {
                        self.update_message_delivery(
                            &pending_message.chat_id,
                            &pending_message.message_id,
                            DeliveryState::Failed,
                        );
                        continue;
                    }
                };

                let payload = match encode_app_direct_message_payload(
                    &pending_message.chat_id,
                    &pending_message.message_id,
                    &pending_message.body,
                ) {
                    Ok(payload) => payload,
                    Err(error) => {
                        self.state.toast = Some(error.to_string());
                        self.update_message_delivery(
                            &pending_message.chat_id,
                            &pending_message.message_id,
                            DeliveryState::Failed,
                        );
                        continue;
                    }
                };

                let logged_in = self.logged_in.as_mut().expect("checked above");
                let mut rng = OsRng;
                let mut ctx = ProtocolContext::new(now, &mut rng);
                logged_in
                    .session_manager
                    .prepare_send(&mut ctx, owner, payload)
            };

            match prepared {
                Ok(prepared) => {
                    if let Some(reason) = pending_reason_from_prepared(&prepared) {
                        self.push_debug_log(
                            "retry.direct.pending",
                            format!(
                                "chat_id={} reason={reason:?} gaps={}",
                                pending_message.chat_id,
                                summarize_relay_gaps(&prepared.relay_gaps)
                            ),
                        );
                        pending_message.reason = reason.clone();
                        pending_message.next_retry_at_secs =
                            now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                        self.update_message_delivery(
                            &pending_message.chat_id,
                            &pending_message.message_id,
                            DeliveryState::Queued,
                        );
                        self.nudge_protocol_state_for_pending_reason(&reason);
                        pending_message.publish_mode = OutboundPublishMode::WaitForPeer;
                        still_pending.push(pending_message);
                    } else {
                        match build_prepared_publish_batch(
                            &prepared,
                            pending_message.expires_at_secs,
                        ) {
                            Ok(Some(batch)) => {
                                pending_message.publish_mode = publish_mode_for_batch(&batch);
                                pending_message.prepared_publish = Some(batch.clone());
                                pending_message.reason =
                                    pending_reason_for_publish_mode(&pending_message.publish_mode);
                                pending_message.next_retry_at_secs =
                                    retry_deadline_for_publish_mode(
                                        now.get(),
                                        &pending_message.publish_mode,
                                    );
                                pending_message.in_flight = true;
                                self.update_message_delivery(
                                    &pending_message.chat_id,
                                    &pending_message.message_id,
                                    DeliveryState::Pending,
                                );
                                self.start_publish_for_pending(
                                    pending_message.message_id.clone(),
                                    pending_message.chat_id.clone(),
                                    pending_message.publish_mode.clone(),
                                    batch,
                                );
                                still_pending.push(pending_message);
                            }
                            Ok(None) => {
                                pending_message.publish_mode = OutboundPublishMode::WaitForPeer;
                                pending_message.reason = PendingSendReason::MissingDeviceInvite;
                                pending_message.next_retry_at_secs =
                                    now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                                self.update_message_delivery(
                                    &pending_message.chat_id,
                                    &pending_message.message_id,
                                    DeliveryState::Queued,
                                );
                                self.push_debug_log(
                                    "retry.direct.pending",
                                    format!(
                                        "chat_id={} reason={:?}",
                                        pending_message.chat_id, pending_message.reason
                                    ),
                                );
                                self.nudge_protocol_state_for_pending_reason(
                                    &pending_message.reason,
                                );
                                still_pending.push(pending_message);
                            }
                            Err(error) => {
                                self.state.toast = Some(error.to_string());
                                self.update_message_delivery(
                                    &pending_message.chat_id,
                                    &pending_message.message_id,
                                    DeliveryState::Failed,
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    self.state.toast = Some(error.to_string());
                    self.update_message_delivery(
                        &pending_message.chat_id,
                        &pending_message.message_id,
                        DeliveryState::Failed,
                    );
                }
            }
        }

        self.pending_outbound = still_pending;
        self.schedule_next_pending_retry(now.get());
    }

    pub(super) fn queue_pending_outbound(
        &mut self,
        message_id: String,
        chat_id: String,
        body: String,
        expires_at_secs: Option<u64>,
        prepared_publish: Option<PreparedPublishBatch>,
        publish_mode: OutboundPublishMode,
        reason: PendingSendReason,
        next_retry_at_secs: u64,
    ) {
        self.pending_outbound.push(PendingOutbound {
            message_id,
            chat_id,
            body,
            expires_at_secs,
            prepared_publish,
            publish_mode,
            reason,
            next_retry_at_secs,
            in_flight: false,
        });
    }

    pub(super) fn set_pending_outbound_in_flight(&mut self, message_id: &str, in_flight: bool) {
        if let Some(pending) = self
            .pending_outbound
            .iter_mut()
            .find(|pending| pending.message_id == message_id)
        {
            pending.in_flight = in_flight;
        }
    }

    pub(super) fn update_message_delivery(
        &mut self,
        chat_id: &str,
        message_id: &str,
        delivery: DeliveryState,
    ) {
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        if let Some(message) = thread
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            message.delivery = delivery;
        }
    }

    pub(super) fn push_outgoing_message(
        &mut self,
        chat_id: &str,
        body: String,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
        delivery: DeliveryState,
    ) -> ChatMessageSnapshot {
        let message_id = self.allocate_message_id();
        self.push_outgoing_message_with_id(
            message_id,
            chat_id,
            body,
            created_at_secs,
            expires_at_secs,
            delivery,
        )
    }

    pub(super) fn push_outgoing_message_with_id(
        &mut self,
        message_id: String,
        chat_id: &str,
        body: String,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
        delivery: DeliveryState,
    ) -> ChatMessageSnapshot {
        let (body, attachments) = extract_message_attachments(&body);
        let message = ChatMessageSnapshot {
            id: message_id,
            chat_id: chat_id.to_string(),
            author: self
                .state
                .account
                .as_ref()
                .map(|account| account.display_name.clone())
                .unwrap_or_else(|| "me".to_string()),
            body,
            attachments,
            reactions: Vec::new(),
            is_outgoing: true,
            created_at_secs,
            expires_at_secs,
            delivery,
        };
        self.threads
            .entry(chat_id.to_string())
            .or_insert_with(|| ThreadRecord {
                chat_id: chat_id.to_string(),
                unread_count: 0,
                updated_at_secs: created_at_secs,
                messages: Vec::new(),
            })
            .insert_message_sorted(message.clone());
        if let Some(thread) = self.threads.get_mut(chat_id) {
            thread.updated_at_secs = thread.updated_at_secs.max(created_at_secs);
        }
        message
    }

    pub(super) fn push_incoming_message_from(
        &mut self,
        chat_id: &str,
        message_id: Option<String>,
        body: String,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
        author: Option<String>,
    ) {
        let message_id = message_id.unwrap_or_else(|| self.allocate_message_id());
        let author = author.unwrap_or_else(|| self.owner_display_label(chat_id));
        let thread = self
            .threads
            .entry(chat_id.to_string())
            .or_insert_with(|| ThreadRecord {
                chat_id: chat_id.to_string(),
                unread_count: 0,
                updated_at_secs: created_at_secs,
                messages: Vec::new(),
            });
        if self.active_chat_id.as_deref() != Some(chat_id) {
            thread.unread_count = thread.unread_count.saturating_add(1);
        }
        thread.updated_at_secs = thread.updated_at_secs.max(created_at_secs);
        let (body, attachments) = extract_message_attachments(&body);
        thread.insert_message_sorted(ChatMessageSnapshot {
            id: message_id,
            chat_id: chat_id.to_string(),
            author,
            body,
            attachments,
            reactions: Vec::new(),
            is_outgoing: false,
            created_at_secs,
            expires_at_secs,
            delivery: DeliveryState::Received,
        });
    }

    pub(super) fn push_system_notice(&mut self, chat_id: &str, body: String, created_at_secs: u64) {
        let message_id = self.allocate_message_id();
        let thread = self
            .threads
            .entry(chat_id.to_string())
            .or_insert_with(|| ThreadRecord {
                chat_id: chat_id.to_string(),
                unread_count: 0,
                updated_at_secs: created_at_secs,
                messages: Vec::new(),
            });
        if thread
            .messages
            .iter()
            .any(|message| message.author == "Iris" && message.body == body)
        {
            return;
        }
        if self.active_chat_id.as_deref() != Some(chat_id) {
            thread.unread_count = thread.unread_count.saturating_add(1);
        }
        thread.updated_at_secs = thread.updated_at_secs.max(created_at_secs);
        thread.insert_message_sorted(ChatMessageSnapshot {
            id: message_id,
            chat_id: chat_id.to_string(),
            author: "Iris".to_string(),
            body,
            attachments: Vec::new(),
            reactions: Vec::new(),
            is_outgoing: false,
            created_at_secs,
            expires_at_secs: None,
            delivery: DeliveryState::Received,
        });
    }

    pub(super) fn toggle_reaction(&mut self, chat_id: &str, message_id: &str, emoji: &str) {
        let emoji = emoji.trim();
        if chat_id.is_empty() || message_id.is_empty() || emoji.is_empty() {
            return;
        }
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
        toggle_local_reaction(message, emoji);
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn delete_local_message(&mut self, chat_id: &str, message_id: &str) {
        if chat_id.is_empty() || message_id.is_empty() {
            return;
        }
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        let original_len = thread.messages.len();
        thread.messages.retain(|message| message.id != message_id);
        if thread.messages.len() == original_len {
            return;
        }
        thread.updated_at_secs = thread
            .messages
            .last()
            .map(|message| message.created_at_secs)
            .unwrap_or(thread.updated_at_secs);
        if self.active_chat_id.as_deref() == Some(chat_id) {
            thread.unread_count = 0;
        }
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn send_typing(&mut self, chat_id: &str) {
        if !self.preferences.send_typing_indicators {
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        if is_group_chat_id(&normalized_chat_id) {
            self.send_group_control(&normalized_chat_id, AppControlType::Typing, Vec::new());
        } else {
            self.send_direct_control(&normalized_chat_id, AppControlType::Typing, Vec::new());
        }
    }

    pub(super) fn set_typing_indicators_enabled(&mut self, enabled: bool) {
        if self.preferences.send_typing_indicators == enabled {
            return;
        }
        self.preferences.send_typing_indicators = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_read_receipts_enabled(&mut self, enabled: bool) {
        if self.preferences.send_read_receipts == enabled {
            return;
        }
        self.preferences.send_read_receipts = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_chat_message_ttl(&mut self, chat_id: &str, ttl_seconds: Option<u64>) {
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        match ttl_seconds {
            Some(ttl_seconds) if ttl_seconds > 0 => {
                self.chat_message_ttl_seconds
                    .insert(normalized_chat_id, ttl_seconds);
            }
            _ => {
                self.chat_message_ttl_seconds.remove(&normalized_chat_id);
            }
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_desktop_notifications_enabled(&mut self, enabled: bool) {
        if self.preferences.desktop_notifications_enabled == enabled {
            return;
        }
        self.preferences.desktop_notifications_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_startup_at_login_enabled(&mut self, enabled: bool) {
        if self.preferences.startup_at_login_enabled == enabled {
            return;
        }
        self.preferences.startup_at_login_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn add_nostr_relay(&mut self, relay_url: &str) {
        let normalized = match normalize_nostr_relay_url(relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        if self.preferences.nostr_relay_urls.contains(&normalized) {
            return self.reject_relay_setting("Relay already exists.".to_string());
        }

        let mut next = self.preferences.nostr_relay_urls.clone();
        next.push(normalized);
        self.apply_nostr_relay_urls(next);
    }

    pub(super) fn update_nostr_relay(&mut self, old_relay_url: &str, new_relay_url: &str) {
        let old_normalized = match normalize_nostr_relay_url(old_relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        let new_normalized = match normalize_nostr_relay_url(new_relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        let Some(index) = self
            .preferences
            .nostr_relay_urls
            .iter()
            .position(|relay| relay == &old_normalized)
        else {
            return self.reject_relay_setting("Relay not found.".to_string());
        };
        if old_normalized != new_normalized
            && self.preferences.nostr_relay_urls.contains(&new_normalized)
        {
            return self.reject_relay_setting("Relay already exists.".to_string());
        }

        let mut next = self.preferences.nostr_relay_urls.clone();
        next[index] = new_normalized;
        self.apply_nostr_relay_urls(next);
    }

    pub(super) fn remove_nostr_relay(&mut self, relay_url: &str) {
        let normalized = match normalize_nostr_relay_url(relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        if self.preferences.nostr_relay_urls.len() <= 1 {
            return self.reject_relay_setting("At least one relay is required.".to_string());
        }
        let Some(index) = self
            .preferences
            .nostr_relay_urls
            .iter()
            .position(|relay| relay == &normalized)
        else {
            return self.reject_relay_setting("Relay not found.".to_string());
        };

        let mut next = self.preferences.nostr_relay_urls.clone();
        next.remove(index);
        self.apply_nostr_relay_urls(next);
    }

    pub(super) fn reset_nostr_relays(&mut self) {
        self.apply_nostr_relay_urls(configured_relays());
    }

    pub(super) fn set_image_proxy_enabled(&mut self, enabled: bool) {
        if self.preferences.image_proxy_enabled == enabled {
            return;
        }
        self.preferences.image_proxy_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_image_proxy_url(&mut self, url: &str) {
        let normalized = normalized_setting(url, crate::image_proxy::DEFAULT_IMAGE_PROXY_URL);
        if self.preferences.image_proxy_url == normalized {
            return;
        }
        self.preferences.image_proxy_url = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_image_proxy_key_hex(&mut self, key_hex: &str) {
        let normalized = normalized_setting(
            &key_hex.to_ascii_lowercase(),
            crate::image_proxy::DEFAULT_IMAGE_PROXY_KEY_HEX,
        );
        if self.preferences.image_proxy_key_hex == normalized {
            return;
        }
        self.preferences.image_proxy_key_hex = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_image_proxy_salt_hex(&mut self, salt_hex: &str) {
        let normalized = normalized_setting(
            &salt_hex.to_ascii_lowercase(),
            crate::image_proxy::DEFAULT_IMAGE_PROXY_SALT_HEX,
        );
        if self.preferences.image_proxy_salt_hex == normalized {
            return;
        }
        self.preferences.image_proxy_salt_hex = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn reset_image_proxy_settings(&mut self) {
        self.preferences.image_proxy_enabled = true;
        self.preferences.image_proxy_url = crate::image_proxy::DEFAULT_IMAGE_PROXY_URL.to_string();
        self.preferences.image_proxy_key_hex =
            crate::image_proxy::DEFAULT_IMAGE_PROXY_KEY_HEX.to_string();
        self.preferences.image_proxy_salt_hex =
            crate::image_proxy::DEFAULT_IMAGE_PROXY_SALT_HEX.to_string();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn apply_nostr_relay_urls(&mut self, relay_urls: Vec<String>) {
        let normalized = normalize_nostr_relay_urls(&relay_urls);
        if self.preferences.nostr_relay_urls == normalized {
            return;
        }

        self.preferences.nostr_relay_urls = normalized;
        let next_relay_urls = relay_urls_from_strings(&self.preferences.nostr_relay_urls);
        let should_refresh = if let Some(logged_in) = self.logged_in.as_mut() {
            let client = logged_in.client.clone();
            let previous_relay_urls = logged_in.relay_urls.clone();
            self.runtime.block_on(sync_session_relays(
                &client,
                &previous_relay_urls,
                &next_relay_urls,
            ));
            logged_in.relay_urls = next_relay_urls;
            true
        } else {
            false
        };

        if should_refresh {
            self.schedule_session_connect();
            self.request_protocol_subscription_refresh_forced();
            self.fetch_recent_protocol_state();
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn reject_relay_setting(&mut self, message: String) {
        self.state.toast = Some(message);
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
        if is_group_chat_id(&normalized_chat_id) {
            // Group read state is local-only for now, matching the Flutter client.
        } else if self.preferences.send_read_receipts {
            self.send_direct_control(&normalized_chat_id, AppControlType::Seen, receipt_ids);
        }

        if changed {
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
        }
    }

    pub(super) fn send_direct_control(
        &mut self,
        chat_id: &str,
        control_type: AppControlType,
        message_ids: Vec<String>,
    ) {
        let Ok((normalized_chat_id, peer_pubkey)) = parse_peer_input(chat_id) else {
            return;
        };
        let payload = match encode_app_control_payload(
            control_type,
            Some(normalized_chat_id.clone()),
            message_ids,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                self.push_debug_log("control.direct.encode", error.to_string());
                return;
            }
        };
        let Some(logged_in) = self.logged_in.as_mut() else {
            return;
        };
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(unix_now(), &mut rng);
        let owner = OwnerPubkey::from_bytes(peer_pubkey.to_bytes());
        match logged_in
            .session_manager
            .prepare_send(&mut ctx, owner, payload)
        {
            Ok(prepared) => match build_prepared_publish_batch(&prepared, None) {
                Ok(Some(batch)) => {
                    let control_id = format!("control-{}", self.allocate_message_id());
                    let publish_mode = publish_mode_for_batch(&batch);
                    self.start_publish_for_pending(
                        control_id,
                        normalized_chat_id,
                        publish_mode,
                        batch,
                    );
                }
                Ok(None) => {}
                Err(error) => self.push_debug_log("control.direct.publish", error.to_string()),
            },
            Err(error) => self.push_debug_log("control.direct.prepare", error.to_string()),
        }
    }

    pub(super) fn send_group_control(
        &mut self,
        chat_id: &str,
        control_type: AppControlType,
        message_ids: Vec<String>,
    ) {
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            return;
        };
        let payload = match encode_app_control_payload(control_type, None, message_ids) {
            Ok(payload) => payload,
            Err(error) => {
                self.push_debug_log("control.group.encode", error.to_string());
                return;
            }
        };
        let Some(logged_in) = self.logged_in.as_mut() else {
            return;
        };
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(unix_now(), &mut rng);
        let (session_manager, group_manager) =
            (&mut logged_in.session_manager, &mut logged_in.group_manager);
        match group_manager.send_message(session_manager, &mut ctx, &group_id, payload) {
            Ok(prepared) => {
                self.publish_group_local_sibling_best_effort(&prepared);
                match build_group_prepared_publish_batch(&prepared, None) {
                    Ok(Some(batch)) => {
                        let control_id = format!("control-{}", self.allocate_message_id());
                        let publish_mode = publish_mode_for_batch(&batch);
                        self.start_publish_for_pending(
                            control_id,
                            chat_id.to_string(),
                            publish_mode,
                            batch,
                        );
                    }
                    Ok(None) => {}
                    Err(error) => self.push_debug_log("control.group.publish", error.to_string()),
                }
            }
            Err(error) => self.push_debug_log("control.group.prepare", error.to_string()),
        }
    }

    pub(super) fn apply_receipt_to_messages(
        &mut self,
        chat_id: &str,
        message_ids: &[String],
        delivery: DeliveryState,
        is_from_local_owner: bool,
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
        }
        if is_from_local_owner && matches!(delivery, DeliveryState::Seen) {
            thread.unread_count = 0;
            changed = true;
        }
        if changed {
            self.persist_best_effort();
        }
    }

    pub(super) fn set_typing_indicator(
        &mut self,
        chat_id: String,
        author_owner_hex: String,
        event_secs: u64,
    ) {
        let expires_at_secs = unix_now().get().saturating_add(TYPING_INDICATOR_TTL_SECS);
        let key = typing_indicator_key(&chat_id, &author_owner_hex);
        self.typing_indicators.insert(
            key,
            TypingIndicatorRecord {
                chat_id: chat_id.clone(),
                author_owner_hex: author_owner_hex.clone(),
                expires_at_secs,
                last_event_secs: event_secs,
            },
        );
        self.schedule_typing_indicator_expiry(chat_id, author_owner_hex);
    }

    pub(super) fn clear_typing_indicator(&mut self, chat_id: &str, author_owner_hex: &str) {
        self.typing_indicators
            .remove(&typing_indicator_key(chat_id, author_owner_hex));
    }

    pub(super) fn schedule_typing_indicator_expiry(&self, chat_id: String, author: String) {
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_secs(TYPING_INDICATOR_TTL_SECS)).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::TypingIndicatorExpired { chat_id, author },
            )));
        });
    }

    pub(super) fn apply_routed_chat_message(
        &mut self,
        routed: RoutedChatMessage,
        created_at_secs: u64,
    ) {
        if routed.is_outgoing {
            match routed.message_id {
                Some(message_id) => self.push_outgoing_message_with_id(
                    message_id,
                    &routed.chat_id,
                    routed.body,
                    created_at_secs,
                    routed.expires_at_secs,
                    DeliveryState::Sent,
                ),
                None => self.push_outgoing_message(
                    &routed.chat_id,
                    routed.body,
                    created_at_secs,
                    routed.expires_at_secs,
                    DeliveryState::Sent,
                ),
            };
        } else {
            self.push_incoming_message_from(
                &routed.chat_id,
                routed.message_id,
                routed.body,
                created_at_secs,
                routed.expires_at_secs,
                routed.author,
            );
        }
    }

    pub(super) fn apply_control_payload(
        &mut self,
        sender_owner: OwnerPubkey,
        control: AppControlPayload,
        created_at_secs: u64,
    ) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let is_from_local_owner = sender_owner == local_owner;
        let chat_id = if is_from_local_owner {
            control
                .chat_id
                .clone()
                .unwrap_or_else(|| sender_owner.to_string())
        } else {
            sender_owner.to_string()
        };

        match control.control_type {
            AppControlType::Typing => {
                if !is_from_local_owner {
                    self.set_typing_indicator(chat_id, sender_owner.to_string(), created_at_secs);
                }
            }
            AppControlType::Delivered => {
                self.apply_receipt_to_messages(
                    &chat_id,
                    &control.message_ids,
                    DeliveryState::Received,
                    is_from_local_owner,
                );
            }
            AppControlType::Seen => {
                self.apply_receipt_to_messages(
                    &chat_id,
                    &control.message_ids,
                    DeliveryState::Seen,
                    is_from_local_owner,
                );
            }
        }
    }

    pub(super) fn route_received_direct_message(
        &self,
        local_owner: OwnerPubkey,
        sender_owner: OwnerPubkey,
        payload: &[u8],
    ) -> RoutedChatMessage {
        match decode_app_payload(payload) {
            AppPayload::DirectMessage(decoded) => {
                if sender_owner == local_owner {
                    if let Ok((chat_id, _)) = parse_peer_input(&decoded.chat_id) {
                        if chat_id != local_owner.to_string() {
                            return RoutedChatMessage {
                                chat_id,
                                message_id: decoded.message_id,
                                body: decoded.body,
                                is_outgoing: true,
                                author: Some(self.owner_display_label(&local_owner.to_string())),
                                expires_at_secs: None,
                            };
                        }
                    }
                }

                RoutedChatMessage {
                    chat_id: sender_owner.to_string(),
                    message_id: decoded.message_id,
                    body: decoded.body,
                    is_outgoing: false,
                    author: Some(self.owner_display_label(&sender_owner.to_string())),
                    expires_at_secs: None,
                }
            }
            AppPayload::LegacyText(body) => RoutedChatMessage {
                chat_id: sender_owner.to_string(),
                message_id: None,
                body,
                is_outgoing: false,
                author: Some(self.owner_display_label(&sender_owner.to_string())),
                expires_at_secs: None,
            },
            AppPayload::GroupMessage(decoded) => RoutedChatMessage {
                chat_id: sender_owner.to_string(),
                message_id: decoded.message_id,
                body: decoded.body,
                is_outgoing: false,
                author: Some(self.owner_display_label(&sender_owner.to_string())),
                expires_at_secs: None,
            },
            AppPayload::Control(_) => RoutedChatMessage {
                chat_id: sender_owner.to_string(),
                message_id: None,
                body: String::new(),
                is_outgoing: false,
                author: Some(self.owner_display_label(&sender_owner.to_string())),
                expires_at_secs: None,
            },
        }
    }

    pub(super) fn apply_group_metadata_update(
        &mut self,
        group: GroupSnapshot,
        previous: Option<GroupSnapshot>,
        created_at_secs: u64,
    ) {
        self.apply_group_snapshot_to_threads_with_notices(
            previous.as_ref(),
            &group,
            created_at_secs.max(group.updated_at.get()),
        );
    }

    pub(super) fn apply_decrypted_payload(
        &mut self,
        sender_owner: OwnerPubkey,
        payload: &[u8],
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
    ) -> anyhow::Result<()> {
        let local_owner = self.logged_in.as_ref().expect("logged in").owner_pubkey;

        let previous_groups = self
            .logged_in
            .as_ref()
            .map(|logged_in| {
                logged_in
                    .group_manager
                    .snapshot()
                    .groups
                    .into_iter()
                    .map(|group| (group.group_id.clone(), group))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        let group_event = {
            let logged_in = self.logged_in.as_mut().expect("logged in");
            logged_in
                .group_manager
                .handle_incoming(sender_owner, payload)?
        };

        match group_event {
            Some(GroupIncomingEvent::MetadataUpdated(group)) => {
                let previous = previous_groups.get(&group.group_id).cloned();
                self.apply_group_metadata_update(group, previous, created_at_secs);
            }
            Some(GroupIncomingEvent::Message(group_message)) => {
                let chat_id = group_chat_id(&group_message.group_id);
                match decode_app_payload(&group_message.body) {
                    AppPayload::Control(control) => {
                        if group_message.sender_owner != local_owner
                            && control.control_type == AppControlType::Typing
                        {
                            self.set_typing_indicator(
                                chat_id,
                                group_message.sender_owner.to_string(),
                                created_at_secs,
                            );
                        }
                    }
                    AppPayload::GroupMessage(decoded) => {
                        self.clear_typing_indicator(
                            &chat_id,
                            &group_message.sender_owner.to_string(),
                        );
                        self.apply_routed_chat_message(
                            RoutedChatMessage {
                                chat_id,
                                message_id: decoded.message_id,
                                body: decoded.body,
                                is_outgoing: group_message.sender_owner == local_owner,
                                author: Some(
                                    self.owner_display_label(
                                        &group_message.sender_owner.to_string(),
                                    ),
                                ),
                                expires_at_secs,
                            },
                            created_at_secs,
                        );
                    }
                    AppPayload::LegacyText(body) => {
                        self.clear_typing_indicator(
                            &chat_id,
                            &group_message.sender_owner.to_string(),
                        );
                        self.apply_routed_chat_message(
                            RoutedChatMessage {
                                chat_id,
                                message_id: None,
                                body,
                                is_outgoing: group_message.sender_owner == local_owner,
                                author: Some(
                                    self.owner_display_label(
                                        &group_message.sender_owner.to_string(),
                                    ),
                                ),
                                expires_at_secs,
                            },
                            created_at_secs,
                        );
                    }
                    AppPayload::DirectMessage(decoded) => {
                        self.clear_typing_indicator(
                            &chat_id,
                            &group_message.sender_owner.to_string(),
                        );
                        self.apply_routed_chat_message(
                            RoutedChatMessage {
                                chat_id,
                                message_id: decoded.message_id,
                                body: decoded.body,
                                is_outgoing: group_message.sender_owner == local_owner,
                                author: Some(
                                    self.owner_display_label(
                                        &group_message.sender_owner.to_string(),
                                    ),
                                ),
                                expires_at_secs,
                            },
                            created_at_secs,
                        );
                    }
                }
            }
            None => match decode_app_payload(payload) {
                AppPayload::Control(control) => {
                    self.apply_control_payload(sender_owner, control, created_at_secs);
                }
                _ => {
                    let mut routed =
                        self.route_received_direct_message(local_owner, sender_owner, payload);
                    routed.expires_at_secs = expires_at_secs;
                    let should_send_delivered = !routed.is_outgoing
                        && routed
                            .message_id
                            .as_ref()
                            .map(|id| !id.is_empty())
                            .unwrap_or(false);
                    let receipt_chat_id = routed.chat_id.clone();
                    let receipt_message_id = routed.message_id.clone();
                    self.clear_typing_indicator(&receipt_chat_id, &sender_owner.to_string());
                    self.apply_routed_chat_message(routed, created_at_secs);
                    if should_send_delivered && self.preferences.send_read_receipts {
                        if let Some(message_id) = receipt_message_id {
                            self.send_direct_control(
                                &receipt_chat_id,
                                AppControlType::Delivered,
                                vec![message_id],
                            );
                        }
                    }
                }
            },
        }

        Ok(())
    }

    pub(super) fn allocate_message_id(&mut self) -> String {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        id.to_string()
    }
}

pub(super) fn toggle_local_reaction(message: &mut ChatMessageSnapshot, emoji: &str) {
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn apply_incoming_reaction(message: &mut ChatMessageSnapshot, emoji: &str) -> bool {
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

pub(super) fn typing_indicator_key(chat_id: &str, author_owner_hex: &str) -> String {
    format!("{chat_id}\n{author_owner_hex}")
}

fn normalized_setting(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

async fn sync_session_relays(client: &Client, previous: &[RelayUrl], next: &[RelayUrl]) {
    let next_set = next.iter().cloned().collect::<HashSet<_>>();
    for relay in previous {
        if !next_set.contains(relay) {
            let _ = client.force_remove_relay(relay.clone()).await;
        }
    }
    ensure_session_relays_configured(client, next).await;
}

pub(super) fn should_advance_delivery(current: &DeliveryState, next: &DeliveryState) -> bool {
    if matches!(
        (current, next),
        (DeliveryState::Pending, DeliveryState::Queued)
            | (DeliveryState::Queued, DeliveryState::Pending)
    ) {
        return true;
    }
    delivery_rank(next) > delivery_rank(current)
}

fn delivery_rank(delivery: &DeliveryState) -> u8 {
    match delivery {
        DeliveryState::Queued | DeliveryState::Pending => 0,
        DeliveryState::Sent => 1,
        DeliveryState::Received => 2,
        DeliveryState::Seen => 3,
        DeliveryState::Failed => 4,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn reaction_notification_body(emoji: &str, target_preview: &str) -> String {
    let emoji = emoji.trim();
    let target_preview = target_preview.trim();
    if target_preview.is_empty() {
        format!("New reaction {emoji}")
    } else {
        format!(
            "Reaction {emoji} to \"{}\"",
            truncate_reaction_preview(target_preview)
        )
    }
}

fn sort_message_reactions(reactions: &mut [MessageReactionSnapshot]) {
    reactions.sort_by(|left, right| {
        left.emoji
            .cmp(&right.emoji)
            .then_with(|| right.count.cmp(&left.count))
    });
}

#[cfg_attr(not(test), allow(dead_code))]
fn truncate_reaction_preview(preview: &str) -> String {
    const MAX_CHARS: usize = 80;
    let mut chars = preview.chars();
    let truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}
