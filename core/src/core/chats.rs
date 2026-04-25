use super::*;

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

        match self.open_direct_chat_from_peer_input(peer_input) {
            Ok(chat_id) => {
                self.push_debug_log(
                    "chat.create",
                    format!("peer_input={} chat_id={chat_id}", peer_input.trim()),
                );
            }
            Err(_) => self.state.toast = Some("Invalid peer key.".to_string()),
        }

        self.rebuild_state();
        self.persist_best_effort();
        self.state.busy.creating_chat = false;
        self.emit_state();
    }

    pub(super) fn open_direct_chat_from_peer_input(
        &mut self,
        peer_input: &str,
    ) -> anyhow::Result<String> {
        let (chat_id, peer_pubkey) = parse_peer_input(peer_input)?;
        let now = unix_now().get();
        self.ensure_thread_record(&chat_id, now).unread_count = 0;

        if let Some(logged_in) = self.logged_in.as_ref() {
            logged_in.ndr_runtime.setup_user(peer_pubkey)?;
        }
        self.process_runtime_events();

        self.active_chat_id = Some(chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }];
        self.republish_local_identity_artifacts();
        self.request_protocol_subscription_refresh();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
        Ok(chat_id)
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

    pub(super) fn find_message_chat_id(&self, message_id: &str) -> Option<String> {
        self.threads
            .iter()
            .find(|(_, thread)| {
                thread
                    .messages
                    .iter()
                    .any(|message| message.id == message_id)
            })
            .map(|(chat_id, _)| chat_id.clone())
    }

    pub(super) fn normalize_chat_id(&self, chat_id: &str) -> Option<String> {
        if is_group_chat_id(chat_id) {
            let group_id = parse_group_id_from_chat_id(chat_id)?;
            let group_chat_id = group_chat_id(&group_id);
            if self.groups.contains_key(&group_id) || self.threads.contains_key(&group_chat_id) {
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

        let now = unix_now();
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

        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let result = logged_in.ndr_runtime.send_text_with_inner_id(
            peer_pubkey,
            text.to_string(),
            send_options_for_expiration(expires_at_secs),
        );

        match result {
            Ok((inner_id, event_ids)) => {
                let message_id = if inner_id.is_empty() {
                    self.allocate_message_id()
                } else {
                    inner_id
                };
                let delivery = if event_ids.is_empty() {
                    DeliveryState::Queued
                } else {
                    DeliveryState::Pending
                };
                self.push_debug_log(
                    "message.direct.send",
                    format!(
                        "chat_id={normalized_chat_id} message_id={message_id} event_ids={}",
                        event_ids.len()
                    ),
                );
                self.push_outgoing_message_with_id(
                    message_id.clone(),
                    &normalized_chat_id,
                    text.to_string(),
                    now.get(),
                    expires_at_secs,
                    delivery,
                );
                let completions = event_ids
                    .iter()
                    .map(|event_id| {
                        (
                            event_id.clone(),
                            (message_id.clone(), normalized_chat_id.clone()),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                self.process_runtime_events_with_completions(&completions);
                if event_ids.is_empty() {
                    self.request_protocol_subscription_refresh();
                }
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
            }
        }
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
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };

        let mut outer_events = Vec::new();
        let mut message_id = None;
        let mut error = None;
        let event = GroupSendEvent {
            kind: CHAT_MESSAGE_KIND,
            content: text.to_string(),
            tags: expires_at_secs
                .and_then(|expires_at| {
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).ok()
                })
                .into_iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect(),
        };
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
                match group_manager.send_event(
                    &group_id,
                    event,
                    &mut send_pairwise,
                    &mut publish_outer,
                    None,
                ) {
                    Ok(result) => {
                        message_id = result
                            .inner
                            .id
                            .as_ref()
                            .map(ToString::to_string)
                            .or_else(|| Some(result.outer.id.to_string()));
                    }
                    Err(send_error) => error = Some(send_error.to_string()),
                }
            });

        match error {
            None => {
                let message_id = message_id.unwrap_or_else(|| self.allocate_message_id());
                self.push_outgoing_message_with_id(
                    message_id.clone(),
                    chat_id,
                    text.to_string(),
                    now.get(),
                    expires_at_secs,
                    DeliveryState::Pending,
                );
                for event in outer_events {
                    self.publish_runtime_event(
                        event,
                        "group message",
                        Some((message_id.clone(), chat_id.to_string())),
                    );
                }
                self.process_runtime_events();
            }
            Some(error) => self.state.toast = Some(error),
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
        if self.threads.get(chat_id).is_some_and(|thread| {
            thread
                .messages
                .iter()
                .any(|message| message.id == message_id)
        }) {
            return;
        }
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

    pub(super) fn send_group_event(
        &mut self,
        chat_id: &str,
        kind: u32,
        content: &str,
        tags: Vec<Vec<String>>,
        now_ms: Option<u64>,
    ) {
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            return;
        };
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let mut outer_events = Vec::new();
        let mut result = Ok(());
        let event = GroupSendEvent {
            kind,
            content: content.to_string(),
            tags,
        };
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
                        now_ms,
                    )
                    .map(|_| ());
            });
        if result.is_ok() {
            for event in outer_events {
                self.publish_runtime_event(event, "group event", None);
            }
            self.process_runtime_events();
        }
    }

    pub(super) fn apply_decrypted_runtime_message(
        &mut self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        content: String,
        outer_event_id: Option<String>,
    ) {
        let Ok(event) = serde_json::from_str::<UnsignedEvent>(&content) else {
            self.apply_runtime_text_message(
                sender_owner,
                None,
                content,
                unix_now().get(),
                None,
                outer_event_id,
            );
            return;
        };

        if let Some(logged_in) = self.logged_in.as_ref() {
            for group_event in logged_in.ndr_runtime.group_handle_incoming_session_event(
                &event,
                sender_owner,
                sender_device,
            ) {
                self.apply_group_decrypted_event(group_event);
            }
        }

        let kind = event.kind.as_u16() as u32;
        let created_at_secs = event.created_at.as_u64();
        let expires_at_secs = message_expiration_from_tags(event.tags.iter());
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let chat_id = chat_id_for_rumor(sender_owner, local_owner, &event);
        let is_outgoing = sender_owner == local_owner;

        match kind {
            GROUP_METADATA_KIND => {
                self.apply_group_metadata_rumor(sender_owner, &event);
            }
            GROUP_SENDER_KEY_DISTRIBUTION_KIND => {}
            CHAT_MESSAGE_KIND => {
                self.apply_runtime_text_message(
                    sender_owner,
                    Some(chat_id.clone()),
                    event.content.clone(),
                    created_at_secs,
                    expires_at_secs,
                    outer_event_id.clone(),
                );
                if !is_outgoing && self.preferences.send_read_receipts {
                    let receipt_id = event
                        .id
                        .as_ref()
                        .map(|id| id.to_string())
                        .or(outer_event_id);
                    if let Some(receipt_id) = receipt_id {
                        self.send_receipt(&chat_id, "delivered", vec![receipt_id]);
                    }
                }
            }
            REACTION_KIND => {
                for message_id in event_message_ids(&event) {
                    self.apply_incoming_reaction_to_chat(&chat_id, &message_id, &event.content);
                }
            }
            RECEIPT_KIND => {
                let delivery = match event.content.as_str() {
                    "seen" => DeliveryState::Seen,
                    _ => DeliveryState::Received,
                };
                self.apply_receipt_to_messages(
                    &chat_id,
                    &event_message_ids(&event),
                    delivery,
                    is_outgoing,
                );
            }
            TYPING_KIND => {
                if !is_outgoing {
                    self.set_typing_indicator(chat_id, sender_owner.to_hex(), created_at_secs);
                }
            }
            CHAT_SETTINGS_KIND => {
                let actor = self.owner_display_label(&sender_owner.to_hex());
                self.apply_chat_settings_control(
                    &chat_id,
                    &actor,
                    chat_settings_ttl_seconds(&event.content),
                    created_at_secs,
                );
            }
            _ => {}
        }
    }

    pub(super) fn apply_runtime_text_message(
        &mut self,
        sender_owner: PublicKey,
        chat_id: Option<String>,
        body: String,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
        message_id: Option<String>,
    ) {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let chat_id = chat_id.unwrap_or_else(|| sender_owner.to_hex());
        self.clear_typing_indicator(&chat_id, &sender_owner.to_hex());
        if sender_owner == local_owner {
            if let Some(message_id) = message_id {
                self.update_message_delivery(&chat_id, &message_id, DeliveryState::Sent);
            }
            return;
        }
        self.push_incoming_message_from(
            &chat_id,
            message_id,
            body,
            created_at_secs,
            expires_at_secs,
            Some(self.owner_display_label(&sender_owner.to_hex())),
        );
    }

    pub(super) fn allocate_message_id(&mut self) -> String {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        id.to_string()
    }
}
