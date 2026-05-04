use super::*;

const OPEN_CHAT_MESSAGES_PER_PAGE: usize = 80;

impl AppCore {
    pub(super) fn create_chat(&mut self, peer_input: &str) {
        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore a profile first.".to_string());
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
        self.prune_expired_messages(now);
        self.ensure_thread_record(&chat_id, now).unread_count = 0;
        self.load_latest_message_page_for_chat(&chat_id);

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
        self.fetch_recent_protocol_state();
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
        self.prune_expired_messages(now);
        self.ensure_thread_record(&chat_id, now).unread_count = 0;
        self.load_latest_message_page_for_chat(&chat_id);
        self.active_chat_id = Some(chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }];
        self.republish_local_identity_artifacts();
        self.rebuild_state();
        self.persist_best_effort();
        self.request_protocol_subscription_refresh();
        self.fetch_recent_protocol_state();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
        self.emit_state();
    }

    fn load_latest_message_page_for_chat(&mut self, chat_id: &str) {
        let messages = match self
            .app_store
            .load_recent_messages(chat_id, OPEN_CHAT_MESSAGES_PER_PAGE)
        {
            Ok(messages) => messages,
            Err(error) => {
                self.push_debug_log(
                    "storage.messages.page.error",
                    format!("chat_id={chat_id} error={error}"),
                );
                return;
            }
        };
        if messages.is_empty() {
            return;
        }
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        let mut page = messages
            .iter()
            .map(chat_message_from_persisted)
            .collect::<Vec<_>>();
        let mut seen = page
            .iter()
            .map(|message| message.id.clone())
            .collect::<HashSet<_>>();
        for message in std::mem::take(&mut thread.messages) {
            if seen.insert(message.id.clone()) {
                page.push(message);
            }
        }
        page.sort_by(|left, right| message_order(left).cmp(&message_order(right)));
        thread.messages = page;
    }

    pub(super) fn send_message(&mut self, chat_id: &str, text: &str, expires_at_secs: Option<u64>) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore a profile first.".to_string());
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
                self.sync_message_delivery_trace(&normalized_chat_id, &message_id);
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
            .with_group_context(|_, group_manager, _| {
                let mut send_pairwise = |recipient: PublicKey, rumor: &UnsignedEvent| {
                    logged_in
                        .ndr_runtime
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
                self.sync_message_delivery_trace(chat_id, &message_id);
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
            message.delivery = delivery.clone();
            if matches!(delivery, DeliveryState::Sent) {
                for recipient in &mut message.recipient_deliveries {
                    if matches!(
                        recipient.delivery,
                        DeliveryState::Pending | DeliveryState::Queued
                    ) {
                        recipient.delivery = DeliveryState::Sent;
                        recipient.updated_at_secs = unix_now().get();
                    }
                }
            }
        }
    }

    pub(super) fn record_message_outer_event(
        &mut self,
        chat_id: &str,
        message_id: &str,
        event_id: &str,
        target_device_id: Option<&str>,
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
        push_unique(&mut message.delivery_trace.outer_event_ids, event_id);
        push_unique(
            &mut message.delivery_trace.pending_relay_event_ids,
            event_id,
        );
        if let Some(target_device_id) = target_device_id {
            push_unique(
                &mut message.delivery_trace.target_device_ids,
                target_device_id,
            );
        }
    }

    pub(super) fn add_message_transport_channel(
        &mut self,
        chat_id: &str,
        message_id: &str,
        channel: &str,
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
        push_unique(&mut message.delivery_trace.transport_channels, channel);
    }

    pub(super) fn add_transport_channel_for_event_id(&mut self, event_id: &str, channel: &str) {
        for thread in self.threads.values_mut() {
            for message in &mut thread.messages {
                let matches_source = message.source_event_id.as_deref() == Some(event_id);
                let matches_outer = message
                    .delivery_trace
                    .outer_event_ids
                    .iter()
                    .any(|outer_event_id| outer_event_id == event_id);
                if matches_source || matches_outer {
                    push_unique(&mut message.delivery_trace.transport_channels, channel);
                }
            }
        }
    }

    pub(super) fn sync_message_delivery_trace(&mut self, chat_id: &str, message_id: &str) {
        let pending_relay_event_ids = self
            .pending_relay_publishes
            .values()
            .filter(|pending| {
                pending.chat_id.as_deref() == Some(chat_id)
                    && pending.message_id.as_deref() == Some(message_id)
            })
            .map(|pending| pending.event_id.clone())
            .collect::<Vec<_>>();
        let last_transport_error = self
            .pending_relay_publishes
            .values()
            .filter(|pending| {
                pending.chat_id.as_deref() == Some(chat_id)
                    && pending.message_id.as_deref() == Some(message_id)
            })
            .filter_map(|pending| pending.last_error.clone())
            .last();
        let queued_protocol_targets = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| {
                logged_in
                    .ndr_runtime
                    .queued_message_diagnostics(Some(message_id))
                    .ok()
            })
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| entry.target_key)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

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
        message.delivery_trace.pending_relay_event_ids = pending_relay_event_ids;
        message.delivery_trace.queued_protocol_targets = queued_protocol_targets;
        message.delivery_trace.last_transport_error = last_transport_error;
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
            kind: ChatMessageKind::User,
            author: self
                .state
                .account
                .as_ref()
                .map(|account| account.display_name.clone())
                .unwrap_or_else(|| "me".to_string()),
            body,
            attachments,
            reactions: Vec::new(),
            reactors: Vec::new(),
            is_outgoing: true,
            created_at_secs,
            expires_at_secs,
            delivery,
            recipient_deliveries: self.initial_recipient_deliveries(chat_id, created_at_secs),
            delivery_trace: MessageDeliveryTraceSnapshot::default(),
            // Outgoing messages are composed locally; the wrapper event
            // id only exists once the rumor has been published, and we
            // never need it for notification preview lookups (we
            // wouldn't notify ourselves).
            source_event_id: None,
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
        self.bump_typing_floor(chat_id, created_at_secs);
        if expires_at_secs.is_some() {
            self.schedule_next_message_expiry();
        }
        message
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_incoming_message_from(
        &mut self,
        chat_id: &str,
        message_id: Option<String>,
        body: String,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
        author: Option<String>,
        source_event_id: Option<String>,
    ) {
        let message_id_ref = message_id.as_deref();
        let source_event_id_ref = source_event_id.as_deref();
        if self.threads.get(chat_id).is_some_and(|thread| {
            thread.messages.iter().any(|message| {
                message_id_ref.is_some_and(|id| message.id == id)
                    || source_event_id_ref.is_some_and(|event_id| {
                        message.source_event_id.as_deref() == Some(event_id)
                    })
            })
        }) {
            return;
        }
        match self
            .app_store
            .message_exists(chat_id, message_id_ref, source_event_id_ref)
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => self.push_debug_log(
                "storage.message.exists.error",
                format!("chat_id={chat_id} error={error}"),
            ),
        }
        let message_id = message_id.unwrap_or_else(|| self.allocate_message_id());
        let author = author.unwrap_or_else(|| self.owner_display_label(chat_id));
        let should_count_unread = !self.is_chat_visible(chat_id);
        let (body, attachments) = extract_message_attachments(&body);
        let mut delivery_trace = delivery_trace_for_source_event(source_event_id.as_deref());
        if let Some(channel) = source_event_id
            .as_ref()
            .and_then(|event_id| self.event_transport_channels.remove(event_id))
        {
            push_unique(&mut delivery_trace.transport_channels, &channel);
        }
        let message = ChatMessageSnapshot {
            id: message_id,
            chat_id: chat_id.to_string(),
            kind: ChatMessageKind::User,
            author,
            body,
            attachments,
            reactions: Vec::new(),
            reactors: Vec::new(),
            is_outgoing: false,
            created_at_secs,
            expires_at_secs,
            delivery: DeliveryState::Received,
            recipient_deliveries: Vec::new(),
            delivery_trace,
            source_event_id,
        };
        let (thread_unread_count, thread_updated_at_secs) = {
            let thread = self
                .threads
                .entry(chat_id.to_string())
                .or_insert_with(|| ThreadRecord {
                    chat_id: chat_id.to_string(),
                    unread_count: 0,
                    updated_at_secs: created_at_secs,
                    messages: Vec::new(),
                });
            if should_count_unread {
                thread.unread_count = thread.unread_count.saturating_add(1);
            }
            thread.updated_at_secs = thread.updated_at_secs.max(created_at_secs);
            thread.insert_message_sorted(message.clone());
            (thread.unread_count, thread.updated_at_secs)
        };
        if message.source_event_id.is_some() {
            if let Err(error) = self.app_store.upsert_notification_preview_message(
                chat_id,
                thread_unread_count,
                thread_updated_at_secs,
                &message,
            ) {
                self.push_debug_log(
                    "storage.message.preview_upsert.error",
                    format!("chat_id={chat_id} message_id={} error={error}", message.id),
                );
            }
        }
        self.bump_typing_floor(chat_id, created_at_secs);
        if expires_at_secs.is_some() {
            self.schedule_next_message_expiry();
        }
    }

    pub(super) fn push_system_notice(&mut self, chat_id: &str, body: String, created_at_secs: u64) {
        let message_id = self.allocate_message_id();
        let should_count_unread = !self.is_chat_visible(chat_id);
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
        if should_count_unread {
            thread.unread_count = thread.unread_count.saturating_add(1);
        }
        thread.updated_at_secs = thread.updated_at_secs.max(created_at_secs);
        thread.insert_message_sorted(ChatMessageSnapshot {
            id: message_id,
            chat_id: chat_id.to_string(),
            kind: ChatMessageKind::System,
            author: "Iris".to_string(),
            body,
            attachments: Vec::new(),
            reactions: Vec::new(),
            reactors: Vec::new(),
            is_outgoing: false,
            created_at_secs,
            expires_at_secs: None,
            delivery: DeliveryState::Received,
            recipient_deliveries: Vec::new(),
            delivery_trace: MessageDeliveryTraceSnapshot::default(),
            source_event_id: None,
        });
        self.bump_typing_floor(chat_id, created_at_secs);
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
        if let Err(error) = self.app_store.delete_message(chat_id, message_id) {
            self.push_debug_log(
                "storage.message.delete.error",
                format!("chat_id={chat_id} message_id={message_id} error={error}"),
            );
        }
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn delete_chat(&mut self, chat_id: &str) {
        if chat_id.is_empty() {
            return;
        }
        let normalized = self
            .normalize_chat_id(chat_id)
            .unwrap_or_else(|| chat_id.to_string());
        let removed_thread = self.threads.remove(&normalized).is_some();
        if removed_thread {
            if let Err(error) = self.app_store.delete_thread(&normalized) {
                self.push_debug_log(
                    "storage.thread.delete.error",
                    format!("chat_id={normalized} error={error}"),
                );
            }
        }
        self.chat_message_ttl_seconds.remove(&normalized);
        self.preferences
            .muted_chat_ids
            .retain(|chat_id| chat_id != &normalized);
        self.mark_mobile_push_dirty();
        self.typing_indicators
            .retain(|_, indicator| indicator.chat_id != normalized);
        self.typing_floor_secs.remove(&normalized);

        let removed_group = if let Some(group_id) = parse_group_id_from_chat_id(&normalized) {
            let was_present = self.groups.remove(&group_id).is_some();
            if was_present {
                self.sync_runtime_groups();
            }
            was_present
        } else {
            false
        };

        if !removed_thread && !removed_group {
            return;
        }

        if self.active_chat_id.as_deref() == Some(normalized.as_str()) {
            self.active_chat_id = None;
        }
        self.screen_stack.retain(|screen| match screen {
            Screen::Chat { chat_id } => chat_id != &normalized,
            Screen::GroupDetails { group_id } => {
                parse_group_id_from_chat_id(&normalized).as_deref() != Some(group_id.as_str())
            }
            _ => true,
        });

        self.push_debug_log("chat.delete", normalized);
        self.rebuild_state();
        self.persist_best_effort();
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
            .with_group_context(|_, group_manager, _| {
                let mut send_pairwise = |recipient: PublicKey, rumor: &UnsignedEvent| {
                    logged_in
                        .ndr_runtime
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
        let Some(runtime_rumor) = parse_runtime_rumor(&content) else {
            self.apply_runtime_text_message(
                sender_owner,
                None,
                content,
                unix_now().get(),
                None,
                outer_event_id.clone(),
                outer_event_id,
            );
            return;
        };

        if let (Some(logged_in), Some(event)) =
            (self.logged_in.as_ref(), runtime_rumor.unsigned.as_ref())
        {
            for group_event in logged_in.ndr_runtime.group_handle_incoming_session_event(
                event,
                sender_owner,
                sender_device,
            ) {
                self.apply_group_decrypted_event(group_event);
            }
        }

        let kind = runtime_rumor.kind;
        let created_at_secs = runtime_rumor.created_at_secs;
        let expires_at_secs = message_expiration_from_tags(runtime_rumor.tags.iter());
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let chat_id = chat_id_for_tags(sender_owner, local_owner, runtime_rumor.tags.iter());
        let is_outgoing = sender_owner == local_owner;
        let message_id = runtime_rumor.id.or_else(|| outer_event_id.clone());

        match kind {
            GROUP_METADATA_KIND => {
                if let Some(event) = runtime_rumor.unsigned.as_ref() {
                    self.apply_group_metadata_rumor(sender_owner, event);
                }
            }
            GROUP_SENDER_KEY_DISTRIBUTION_KIND => {}
            CHAT_MESSAGE_KIND => {
                self.apply_runtime_text_message(
                    sender_owner,
                    Some(chat_id.clone()),
                    runtime_rumor.content,
                    created_at_secs,
                    expires_at_secs,
                    message_id.clone(),
                    outer_event_id.clone(),
                );
                if !is_outgoing && self.preferences.send_read_receipts {
                    if let Some(receipt_id) = message_id {
                        self.send_receipt(&chat_id, "delivered", vec![receipt_id]);
                    }
                }
            }
            REACTION_KIND => {
                let sender_hex = sender_owner.to_hex();
                for message_id in message_ids_from_tags(runtime_rumor.tags.iter()) {
                    self.apply_incoming_reaction_to_chat(
                        &chat_id,
                        &message_id,
                        &sender_hex,
                        &runtime_rumor.content,
                    );
                }
            }
            RECEIPT_KIND => {
                let delivery = match runtime_rumor.content.as_str() {
                    "seen" => DeliveryState::Seen,
                    _ => DeliveryState::Received,
                };
                self.apply_receipt_to_messages(
                    &chat_id,
                    &message_ids_from_tags(runtime_rumor.tags.iter()),
                    delivery,
                    is_outgoing,
                    Some(&sender_owner.to_hex()),
                );
            }
            TYPING_KIND => {
                if !is_outgoing {
                    self.apply_typing_event(
                        chat_id,
                        sender_owner.to_hex(),
                        created_at_secs,
                        expires_at_secs,
                    );
                }
            }
            CHAT_SETTINGS_KIND => {
                let actor = self.owner_display_label(&sender_owner.to_hex());
                self.apply_chat_settings_control(
                    &chat_id,
                    &actor,
                    chat_settings_ttl_seconds(&runtime_rumor.content),
                    created_at_secs,
                );
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_runtime_text_message(
        &mut self,
        sender_owner: PublicKey,
        chat_id: Option<String>,
        body: String,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
        message_id: Option<String>,
        source_event_id: Option<String>,
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
            let message_id = message_id.unwrap_or_else(|| self.allocate_message_id());
            if self.threads.get(&chat_id).is_some_and(|thread| {
                thread
                    .messages
                    .iter()
                    .any(|message| message.id == message_id)
            }) {
                self.update_message_delivery(&chat_id, &message_id, DeliveryState::Sent);
            } else {
                self.push_outgoing_message_with_id(
                    message_id,
                    &chat_id,
                    body,
                    created_at_secs,
                    expires_at_secs,
                    DeliveryState::Sent,
                );
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
            source_event_id,
        );
    }

    pub(super) fn allocate_message_id(&mut self) -> String {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        id.to_string()
    }

    fn initial_recipient_deliveries(
        &self,
        chat_id: &str,
        created_at_secs: u64,
    ) -> Vec<MessageRecipientDeliverySnapshot> {
        let local_owner = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex());
        let mut recipients = if let Some(group_id) = parse_group_id_from_chat_id(chat_id) {
            self.groups
                .get(&group_id)
                .map(|group| group.members.clone())
                .unwrap_or_default()
        } else {
            vec![chat_id.to_string()]
        };
        recipients.retain(|owner| local_owner.as_deref() != Some(owner.as_str()));
        recipients.sort();
        recipients.dedup();
        recipients
            .into_iter()
            .map(|owner_pubkey_hex| MessageRecipientDeliverySnapshot {
                owner_pubkey_hex,
                delivery: DeliveryState::Pending,
                updated_at_secs: created_at_secs,
            })
            .collect()
    }
}

fn chat_message_from_persisted(message: &PersistedMessage) -> ChatMessageSnapshot {
    let (body, parsed_attachments) = extract_message_attachments(&message.body);
    ChatMessageSnapshot {
        id: message.id.clone(),
        chat_id: message.chat_id.clone(),
        kind: message.kind.clone(),
        author: message.author.clone(),
        body,
        attachments: if message.attachments.is_empty() {
            parsed_attachments
        } else {
            message.attachments.clone()
        },
        reactions: message.reactions.clone(),
        reactors: message.reactors.clone(),
        is_outgoing: message.is_outgoing,
        created_at_secs: message.created_at_secs,
        expires_at_secs: message.expires_at_secs,
        delivery: message.delivery.clone().into(),
        recipient_deliveries: message.recipient_deliveries.clone(),
        delivery_trace: message.delivery_trace.clone(),
        source_event_id: message.source_event_id.clone(),
    }
}

fn delivery_trace_for_source_event(source_event_id: Option<&str>) -> MessageDeliveryTraceSnapshot {
    let mut trace = MessageDeliveryTraceSnapshot::default();
    if let Some(source_event_id) = source_event_id {
        trace.outer_event_ids.push(source_event_id.to_string());
    }
    trace
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if values.iter().any(|existing| existing == value) {
        return;
    }
    values.push(value.to_string());
}

fn message_order(message: &ChatMessageSnapshot) -> (u64, u64, &str) {
    (
        message.created_at_secs,
        message.id.parse::<u64>().unwrap_or(u64::MAX),
        message.id.as_str(),
    )
}
