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
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if is_group_chat_id(&normalized_chat_id) {
            self.send_group_event(&normalized_chat_id, TYPING_KIND, "typing", Vec::new(), None);
        } else if let Ok((_, peer)) = parse_peer_input(&normalized_chat_id) {
            let _ = logged_in.ndr_runtime.send_typing(peer, None);
            self.process_runtime_events();
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
        let normalized_ttl = match ttl_seconds {
            Some(ttl_seconds) if ttl_seconds > 0 => {
                self.chat_message_ttl_seconds
                    .insert(normalized_chat_id.clone(), ttl_seconds);
                Some(ttl_seconds)
            }
            _ => {
                self.chat_message_ttl_seconds.remove(&normalized_chat_id);
                None
            }
        };

        let actor = self
            .logged_in
            .as_ref()
            .map(|logged_in| self.owner_display_label(&logged_in.owner_pubkey.to_hex()))
            .unwrap_or_else(|| "You".to_string());
        self.push_system_notice(
            &normalized_chat_id,
            disappearing_timer_notice(&actor, normalized_ttl),
            unix_now().get(),
        );
        if is_group_chat_id(&normalized_chat_id) {
            let content = serde_json::json!({
                "type": "chat-settings",
                "v": 1,
                "messageTtlSeconds": normalized_ttl.unwrap_or(0),
            })
            .to_string();
            self.send_group_event(
                &normalized_chat_id,
                CHAT_SETTINGS_KIND,
                &content,
                Vec::new(),
                None,
            );
        } else if let (Some(logged_in), Ok((_, peer))) = (
            self.logged_in.as_ref(),
            parse_peer_input(&normalized_chat_id),
        ) {
            let ttl = normalized_ttl.unwrap_or(0);
            let _ = logged_in.ndr_runtime.send_chat_settings(peer, ttl);
            self.process_runtime_events();
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
        if self.preferences.send_read_receipts {
            self.send_receipt(&normalized_chat_id, "seen", receipt_ids);
        }

        if changed {
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
        }
    }

    fn send_receipt(&mut self, chat_id: &str, receipt_type: &str, message_ids: Vec<String>) {
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
            let _ = logged_in
                .ndr_runtime
                .send_receipt(peer, receipt_type, message_ids, None);
            self.process_runtime_events();
        }
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

    pub(super) fn apply_chat_settings_control(
        &mut self,
        chat_id: &str,
        actor: &str,
        ttl_seconds: Option<u64>,
        created_at_secs: u64,
    ) {
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        match ttl_seconds {
            Some(ttl_seconds) if ttl_seconds > 0 => {
                self.chat_message_ttl_seconds
                    .insert(normalized_chat_id.clone(), ttl_seconds);
            }
            _ => {
                self.chat_message_ttl_seconds.remove(&normalized_chat_id);
            }
        }
        self.push_system_notice(
            &normalized_chat_id,
            disappearing_timer_notice(actor, ttl_seconds),
            created_at_secs,
        );
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
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

    pub(super) fn allocate_message_id(&mut self) -> String {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        id.to_string()
    }
}

fn disappearing_timer_notice(actor: &str, ttl_seconds: Option<u64>) -> String {
    format!(
        "{actor} set disappearing messages timer to {}",
        disappearing_timer_label(ttl_seconds)
    )
}

fn disappearing_timer_label(ttl_seconds: Option<u64>) -> String {
    match ttl_seconds {
        None | Some(0) => "Off".to_string(),
        Some(300) => "5 minutes".to_string(),
        Some(3600) => "1 hour".to_string(),
        Some(86_400) => "24 hours".to_string(),
        Some(604_800) => "1 week".to_string(),
        Some(2_592_000) => "1 month".to_string(),
        Some(7_776_000) => "3 months".to_string(),
        Some(seconds) if seconds % 86_400 == 0 => {
            let days = seconds / 86_400;
            format!("{days} days")
        }
        Some(seconds) if seconds % 3600 == 0 => {
            let hours = seconds / 3600;
            if hours == 1 {
                "1 hour".to_string()
            } else {
                format!("{hours} hours")
            }
        }
        Some(seconds) if seconds % 60 == 0 => {
            let minutes = seconds / 60;
            if minutes == 1 {
                "1 minute".to_string()
            } else {
                format!("{minutes} minutes")
            }
        }
        Some(seconds) => format!("{seconds} seconds"),
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

impl AppCore {
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

fn sort_message_reactions(reactions: &mut [MessageReactionSnapshot]) {
    reactions.sort_by(|left, right| {
        right
            .reacted_by_me
            .cmp(&left.reacted_by_me)
            .then_with(|| right.count.cmp(&left.count))
            .then_with(|| left.emoji.cmp(&right.emoji))
    });
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
