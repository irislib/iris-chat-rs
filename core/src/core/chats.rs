use super::*;

mod direct_queue;
mod helpers;

use self::direct_queue::is_queued_direct_text_message;
use self::helpers::{
    delivery_trace_for_source_event, is_supported_group_pairwise_payload, push_unique,
    summarize_group_send_effect_targets,
};

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
                self.broadcast_device_sync_snapshot();
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
        let (chat_id, _) = parse_peer_input(peer_input)?;
        let now = unix_now().get();
        self.prune_expired_messages(now);
        self.fetch_missing_profile_metadata(&chat_id, "open_chat");
        self.ensure_thread_record(&chat_id, now).unread_count = 0;
        self.load_latest_message_page_for_chat(&chat_id);

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
                draft: String::new(),
            });
        if thread.updated_at_secs == 0 {
            thread.updated_at_secs = updated_at_secs;
        }
        thread
    }

    pub(super) fn migrate_verified_device_owner_threads(
        &mut self,
        owner: PublicKey,
        app_keys: &AppKeys,
    ) {
        let owner_hex = owner.to_hex();
        for device in app_keys.get_all_devices() {
            let device_hex = device.identity_pubkey.to_hex();
            if device_hex != owner_hex {
                self.migrate_direct_thread_alias(&device_hex, &owner_hex);
            }
        }
    }

    fn migrate_direct_thread_alias(&mut self, from_chat_id: &str, to_chat_id: &str) {
        if from_chat_id == to_chat_id || !self.threads.contains_key(from_chat_id) {
            return;
        }

        let from_label = self.owner_display_label(from_chat_id);
        let to_label = self.owner_display_label(to_chat_id);
        let Some(mut source) = self.threads.remove(from_chat_id) else {
            return;
        };
        let target = self
            .threads
            .entry(to_chat_id.to_string())
            .or_insert_with(|| ThreadRecord {
                chat_id: to_chat_id.to_string(),
                unread_count: 0,
                updated_at_secs: source.updated_at_secs,
                messages: Vec::new(),
                draft: String::new(),
            });

        for mut message in source.messages.drain(..) {
            let duplicate = target.messages.iter().any(|existing| {
                existing.id == message.id
                    || message
                        .source_event_id
                        .as_ref()
                        .is_some_and(|source_event_id| {
                            existing.source_event_id.as_ref() == Some(source_event_id)
                        })
            });
            if duplicate {
                continue;
            }
            message.chat_id = to_chat_id.to_string();
            if !message.is_outgoing
                && (message.author == from_chat_id || message.author == from_label)
            {
                message.author = to_label.clone();
            }
            for delivery in &mut message.recipient_deliveries {
                if delivery.owner_pubkey_hex == from_chat_id {
                    delivery.owner_pubkey_hex = to_chat_id.to_string();
                }
            }
            for reactor in &mut message.reactors {
                if reactor.author == from_chat_id {
                    reactor.author = to_chat_id.to_string();
                }
            }
            target.insert_message_sorted(message);
        }
        target.unread_count = target.unread_count.saturating_add(source.unread_count);
        target.updated_at_secs = target.updated_at_secs.max(source.updated_at_secs);

        if self.active_chat_id.as_deref() == Some(from_chat_id) {
            self.active_chat_id = Some(to_chat_id.to_string());
        }
        for screen in &mut self.screen_stack {
            if let Screen::Chat { chat_id } = screen {
                if chat_id == from_chat_id {
                    *chat_id = to_chat_id.to_string();
                }
            }
        }
        if let Some(ttl) = self.chat_message_ttl_seconds.remove(from_chat_id) {
            self.chat_message_ttl_seconds
                .entry(to_chat_id.to_string())
                .or_insert(ttl);
        }
        for muted in &mut self.preferences.muted_chat_ids {
            if muted == from_chat_id {
                *muted = to_chat_id.to_string();
            }
        }
        self.preferences.muted_chat_ids.sort();
        self.preferences.muted_chat_ids.dedup();
        for pinned in &mut self.preferences.pinned_chat_ids {
            if pinned == from_chat_id {
                *pinned = to_chat_id.to_string();
            }
        }
        self.preferences.pinned_chat_ids.sort();
        self.preferences.pinned_chat_ids.dedup();
        if let Some(floor) = self.typing_floor_secs.remove(from_chat_id) {
            self.typing_floor_secs
                .entry(to_chat_id.to_string())
                .and_modify(|existing| *existing = (*existing).max(floor))
                .or_insert(floor);
        }
        for peer in self.recent_handshake_peers.values_mut() {
            if peer.owner_hex == from_chat_id {
                peer.owner_hex = to_chat_id.to_string();
            }
        }
        let typing_indicators = std::mem::take(&mut self.typing_indicators);
        for (_, mut indicator) in typing_indicators {
            if indicator.chat_id == from_chat_id {
                indicator.chat_id = to_chat_id.to_string();
            }
            if indicator.author_owner_hex == from_chat_id {
                indicator.author_owner_hex = to_chat_id.to_string();
            }
            self.typing_indicators.insert(
                format!("{}\n{}", indicator.chat_id, indicator.author_owner_hex),
                indicator,
            );
        }
        self.mark_mobile_push_dirty();
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

    pub(super) fn set_chat_draft(&mut self, chat_id: &str, text: &str) {
        let Some(chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let next = text.to_string();
        // No-op when nothing changed — keeps the per-keystroke
        // dispatch from rewriting the threads row on every tick when
        // a debounce window expired with no actual edit (e.g., the
        // user pressed and released a modifier key).
        if self
            .threads
            .get(&chat_id)
            .is_some_and(|thread| thread.draft == next)
        {
            return;
        }
        let now = unix_now().get();
        // Stub a thread record for brand-new chats so a first-time
        // draft survives a relaunch even before any messages exist.
        self.ensure_thread_record(&chat_id, now).draft = next;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
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

        // Flip the screen first and emit, then queue the heavy work
        // (subscriptions, identity republish, persist) as a follow-up
        // InternalEvent on the same event loop. Any UI action enqueued
        // in the meantime (back tap, switch chat, …) interleaves
        // between the screen flip and the finalize, so navigation
        // never sits behind a cold chat's tail.
        //
        // The cheap bits — making sure a thread row exists and pulling
        // the latest page of messages out of SQLite — run inline. They
        // matter for the iOS notification-tap path: the NSE writes a
        // preview message row while the app is suspended, but the
        // in-memory `threads` map doesn't know about it, so without
        // this `current_chat` would be `None` on the first emit and
        // the UI would sit on "Loading chat…" until the finalize
        // landed. With the row stubbed + messages loaded here,
        // `current_chat` is populated immediately and the chat paints
        // its history on the same render that flips the screen.
        let now = unix_now().get();
        self.ensure_thread_record(&chat_id, now).unread_count = 0;
        self.load_latest_message_page_for_chat(&chat_id);
        self.active_chat_id = Some(chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }];
        self.rebuild_state();
        self.emit_state();

        let _ = self.core_sender.send(CoreMsg::Internal(Box::new(
            InternalEvent::OpenChatFinalize { chat_id },
        )));
    }

    pub(super) fn open_chat_finalize(&mut self, chat_id: &str) {
        // If the user already navigated away from this chat before
        // the finalize runs, skip the heavy work — there's no
        // point loading messages into a thread the user isn't
        // looking at, and persisting + republishing identity for an
        // open we abandoned is just wasted I/O.
        if !matches!(
            self.screen_stack.last(),
            Some(Screen::Chat { chat_id: current }) if current == chat_id
        ) {
            return;
        }
        let now = unix_now().get();
        self.prune_expired_messages(now);
        // `open_chat` already stubbed the thread and loaded its latest
        // page so the UI could paint without a "Loading chat…" flash;
        // the finalize only needs to handle the rest (republish
        // identity, subscriptions, persist, schedule peer catch-up).
        self.republish_local_identity_artifacts();
        self.persist_best_effort();
        self.request_protocol_subscription_refresh();
        self.fetch_recent_protocol_state();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
        self.rebuild_state();
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
        page.sort_by_key(message_order);
        thread.messages = page;
    }

    fn hydrate_thread_from_storage(&mut self, chat_id: &str) -> bool {
        let persisted = match self
            .app_store
            .load_thread(chat_id, OPEN_CHAT_MESSAGES_PER_PAGE)
        {
            Ok(Some(thread)) => thread,
            Ok(None) => return false,
            Err(error) => {
                self.push_debug_log(
                    "storage.thread.hydrate.error",
                    format!("chat_id={chat_id} error={error}"),
                );
                return false;
            }
        };
        self.merge_persisted_thread(persisted)
    }

    fn merge_persisted_thread(&mut self, persisted: PersistedThread) -> bool {
        let chat_id = persisted.chat_id.clone();
        let mut changed = false;
        let persisted_updated_at_secs = persisted.updated_at_secs.max(
            persisted
                .messages
                .iter()
                .map(|message| message.created_at_secs)
                .max()
                .unwrap_or(0),
        );
        let messages = persisted
            .messages
            .iter()
            .map(chat_message_from_persisted)
            .collect::<Vec<_>>();
        let thread = self.threads.entry(chat_id.clone()).or_insert_with(|| {
            changed = true;
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: persisted.unread_count,
                updated_at_secs: persisted_updated_at_secs,
                messages: Vec::new(),
                draft: persisted.draft.clone(),
            }
        });
        if persisted.unread_count > thread.unread_count {
            thread.unread_count = persisted.unread_count;
            changed = true;
        }
        if persisted_updated_at_secs > thread.updated_at_secs {
            thread.updated_at_secs = persisted_updated_at_secs;
            changed = true;
        }
        if thread.draft.is_empty() && !persisted.draft.is_empty() {
            thread.draft = persisted.draft;
            changed = true;
        }
        for message in messages {
            let duplicate = thread.messages.iter().any(|existing| {
                existing.id == message.id
                    || message
                        .source_event_id
                        .as_ref()
                        .is_some_and(|source_event_id| {
                            existing.source_event_id.as_ref() == Some(source_event_id)
                        })
            });
            if !duplicate {
                thread.insert_message_sorted(message);
                changed = true;
            }
        }
        if let Some(latest) = thread.messages.last() {
            self.typing_floor_secs
                .entry(chat_id)
                .and_modify(|floor| *floor = (*floor).max(latest.created_at_secs))
                .or_insert(latest.created_at_secs);
        }
        changed
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

        if !is_group_chat_id(&normalized_chat_id) && self.is_owner_blocked(&normalized_chat_id) {
            self.state.toast = Some("User is blocked.".to_string());
            self.emit_state();
            return;
        }

        let now = unix_now();
        self.active_chat_id = Some(normalized_chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: normalized_chat_id.clone(),
        }];
        let thread = self.ensure_thread_record(&normalized_chat_id, now.get());
        // The draft is what the user typed and just sent — clear it
        // from the persisted thread so a relaunch doesn't re-fill the
        // composer with text we've already shipped. Matches Signal's
        // `clearTextMessage` + `updateWithDraft(nil)` flow.
        if !thread.draft.is_empty() {
            thread.draft.clear();
        }
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
        let Ok((normalized_chat_id, _peer_pubkey)) = parse_peer_input(chat_id) else {
            self.state.toast = Some("Invalid peer key.".to_string());
            return;
        };

        let message_id = self.allocate_message_id();
        self.push_debug_log(
            "message.direct.queue",
            format!(
                "reason=message.direct.send chat_id={normalized_chat_id} message_id={message_id}"
            ),
        );
        self.push_outgoing_message_with_id(
            message_id,
            &normalized_chat_id,
            text.to_string(),
            now.get(),
            expires_at_secs,
            DeliveryState::Queued,
        );
        self.drain_queued_direct_text_messages("message.direct.send");
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
        let Some(owner_pubkey) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            self.state.toast = Some("Create or restore a profile first.".to_string());
            return;
        };
        let mut tags = Vec::new();
        if let Ok(group_tag) = nostr::Tag::parse(["l", group_id.as_str()]) {
            tags.push(group_tag);
        }
        if let Some(expires_at_secs) = expires_at_secs {
            let expiration = expires_at_secs.to_string();
            if let Ok(expiration_tag) = nostr::Tag::parse(["expiration", expiration.as_str()]) {
                tags.push(expiration_tag);
            }
        }
        let mut rumor = UnsignedEvent::new(
            owner_pubkey,
            Timestamp::from_secs(now.get()),
            Kind::Custom(CHAT_MESSAGE_KIND as u16),
            tags,
            text.to_string(),
        );
        rumor.ensure_id();
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| self.allocate_message_id());
        let payload = match serde_json::to_vec(&rumor) {
            Ok(payload) => payload,
            Err(error) => {
                self.state.toast = Some(error.to_string());
                return;
            }
        };
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.send_group_payload(&group_id, payload, Some(message_id.clone())));
        match result {
            Some(Ok(result)) => {
                let delivery = if result.event_ids.is_empty() {
                    DeliveryState::Queued
                } else {
                    DeliveryState::Pending
                };
                let publish_effects = result
                    .effects
                    .iter()
                    .filter(|effect| matches!(effect, ProtocolEffect::Publish(_)))
                    .count();
                let delivery_publish_effects = result
                    .effects
                    .iter()
                    .filter(|effect| {
                        matches!(
                            effect,
                            ProtocolEffect::Publish(publish) if publish.inner_event_id.is_some()
                        )
                    })
                    .count();
                self.push_debug_log(
                    "message.group.send.appcore",
                    format!(
                        "chat_id={chat_id} message_id={message_id} event_ids={} effects={} signed={} delivery_publish={} targets={}",
                        result.event_ids.len(),
                        result.effects.len(),
                        publish_effects,
                        delivery_publish_effects,
                        summarize_group_send_effect_targets(&result.effects)
                    ),
                );
                self.push_outgoing_message_with_id(
                    message_id.clone(),
                    chat_id,
                    text.to_string(),
                    now.get(),
                    expires_at_secs,
                    delivery,
                );
                self.process_protocol_engine_effects(result.effects);
                self.sync_message_delivery_trace(chat_id, &message_id);
                self.reconcile_outgoing_message_delivery(chat_id, &message_id);
                self.request_protocol_subscription_refresh();
            }
            Some(Err(error)) => self.state.toast = Some(error.to_string()),
            None => self.state.toast = Some("Protocol engine is not ready.".to_string()),
        }
    }

    pub(super) fn update_message_delivery(
        &mut self,
        chat_id: &str,
        message_id: &str,
        delivery: DeliveryState,
    ) {
        let mut device_sync_message = None;
        let Some(thread) = self.threads.get_mut(chat_id) else {
            return;
        };
        if let Some(message) = thread
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            let became_sent = matches!(delivery, DeliveryState::Sent)
                && !matches!(message.delivery, DeliveryState::Sent);
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
            if became_sent {
                device_sync_message = Some(message.clone());
            }
        }
        if let Some(message) = device_sync_message {
            self.broadcast_device_sync_message(&message);
        }
    }

    pub(super) fn record_message_outer_event(
        &mut self,
        chat_id: &str,
        message_id: &str,
        event_id: &str,
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

    /// Returns true if a transport-channel entry was actually added —
    /// callers use this to skip the persist + rebuild + emit cycle
    /// when a duplicate relay event arrives whose channel set was
    /// already recorded, which is the common firehose case once a
    /// chat has been seen on multiple mirrored relays.
    pub(super) fn add_transport_channel_for_event_id(
        &mut self,
        event_id: &str,
        channel: &str,
    ) -> bool {
        let mut changed = false;
        for thread in self.threads.values_mut() {
            for message in &mut thread.messages {
                let matches_source = message.source_event_id.as_deref() == Some(event_id);
                let matches_outer = message
                    .delivery_trace
                    .outer_event_ids
                    .iter()
                    .any(|outer_event_id| outer_event_id == event_id);
                if (matches_source || matches_outer)
                    && !message
                        .delivery_trace
                        .transport_channels
                        .iter()
                        .any(|existing| existing == channel)
                {
                    message
                        .delivery_trace
                        .transport_channels
                        .push(channel.to_string());
                    changed = true;
                }
            }
        }
        changed
    }

    pub(super) fn sync_message_delivery_trace(&mut self, chat_id: &str, message_id: &str) {
        let pending_relay_event_ids = self
            .pending_relay_publishes
            .values()
            .filter(|pending| {
                pending.chat_id.as_deref() == Some(chat_id)
                    && pending.inner_event_id.as_deref() == Some(message_id)
            })
            .map(|pending| pending.event_id.clone())
            .collect::<Vec<_>>();
        let last_transport_error = self
            .pending_relay_publishes
            .values()
            .filter(|pending| {
                pending.chat_id.as_deref() == Some(chat_id)
                    && pending.inner_event_id.as_deref() == Some(message_id)
            })
            .filter_map(|pending| pending.last_error.clone())
            .next_back();
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
        message.delivery_trace.queued_protocol_targets = Vec::new();
        message.delivery_trace.last_transport_error = last_transport_error;
    }

    pub(super) fn reconcile_outgoing_message_delivery(&mut self, chat_id: &str, message_id: &str) {
        let pending_relay = self.pending_relay_publishes.values().any(|pending| {
            pending.chat_id.as_deref() == Some(chat_id)
                && pending.inner_event_id.as_deref() == Some(message_id)
        });
        let queued_protocol = self
            .protocol_engine
            .as_ref()
            .is_some_and(|protocol_engine| {
                protocol_engine.has_delivery_blocking_message_work(message_id)
            });
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
        if !message.is_outgoing || matches!(message.delivery, DeliveryState::Failed) {
            return;
        }
        if is_queued_direct_text_message(chat_id, message) {
            return;
        }
        if pending_relay || queued_protocol {
            if matches!(message.delivery, DeliveryState::Sent) {
                message.delivery = DeliveryState::Pending;
            }
            return;
        }
        if matches!(
            message.delivery,
            DeliveryState::Pending | DeliveryState::Queued
        ) {
            message.delivery = DeliveryState::Sent;
        }
        let now = unix_now().get();
        for recipient in &mut message.recipient_deliveries {
            if matches!(
                recipient.delivery,
                DeliveryState::Pending | DeliveryState::Queued
            ) {
                recipient.delivery = DeliveryState::Sent;
                recipient.updated_at_secs = now;
            }
        }
    }

    pub(super) fn reconcile_ready_outgoing_message_deliveries(&mut self) {
        let message_refs = self
            .threads
            .iter()
            .flat_map(|(chat_id, thread)| {
                thread.messages.iter().filter_map(|message| {
                    if message.is_outgoing
                        && matches!(
                            message.delivery,
                            DeliveryState::Pending | DeliveryState::Queued
                        )
                    {
                        Some((chat_id.clone(), message.id.clone()))
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();

        for (chat_id, message_id) in message_refs {
            self.reconcile_outgoing_message_delivery(&chat_id, &message_id);
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
        let author_owner_pubkey_hex = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex());
        let author_picture_url = author_owner_pubkey_hex
            .as_ref()
            .and_then(|owner_hex| self.owner_picture_url(owner_hex));
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
            author_owner_pubkey_hex,
            author_picture_url,
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
                draft: String::new(),
            })
            .insert_message_sorted(message.clone());
        if let Some(thread) = self.threads.get_mut(chat_id) {
            thread.updated_at_secs = thread.updated_at_secs.max(created_at_secs);
        }
        // Any outgoing message landing in a direct thread — local send
        // OR self-synced reply from a linked device — counts as
        // implicit Accept. Add the peer to the accepted set so the
        // push subscription includes them too (the projection's
        // `is_request` flag already flips off on any outgoing, but the
        // wire layer only sees this set, not message history).
        self.accept_direct_peer(chat_id);
        self.bump_typing_floor(chat_id, created_at_secs);
        if expires_at_secs.is_some() {
            self.schedule_next_message_expiry();
        }
        self.broadcast_device_sync_message(&message);
        message
    }

    pub(super) fn accept_direct_peer(&mut self, chat_id: &str) {
        if !is_group_chat_id(chat_id)
            && !self
                .preferences
                .accepted_owner_pubkeys
                .iter()
                .any(|hex| hex == chat_id)
        {
            self.preferences
                .accepted_owner_pubkeys
                .push(chat_id.to_string());
            self.preferences.accepted_owner_pubkeys.sort();
            self.preferences.accepted_owner_pubkeys.dedup();
            self.mark_mobile_push_dirty();
        }
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
        author_owner_pubkey_hex: Option<String>,
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
            Ok(true) => {
                if !self.threads.contains_key(chat_id) {
                    self.hydrate_thread_from_storage(chat_id);
                }
                return;
            }
            Ok(false) => {}
            Err(error) => self.push_debug_log(
                "storage.message.exists.error",
                format!("chat_id={chat_id} error={error}"),
            ),
        }
        let message_id = message_id.unwrap_or_else(|| self.allocate_message_id());
        let author_owner_pubkey_hex = author_owner_pubkey_hex
            .or_else(|| (!is_group_chat_id(chat_id)).then(|| chat_id.to_string()));
        if let Some(owner_hex) = author_owner_pubkey_hex.as_deref() {
            self.fetch_missing_profile_metadata(owner_hex, "incoming_message");
        }
        let author_picture_url = author_owner_pubkey_hex
            .as_ref()
            .and_then(|owner_hex| self.owner_picture_url(owner_hex));
        let author = author.unwrap_or_else(|| {
            author_owner_pubkey_hex
                .as_deref()
                .map(|owner| self.owner_display_label(owner))
                .unwrap_or_else(|| self.owner_display_label(chat_id))
        });
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
            author_owner_pubkey_hex,
            author_picture_url,
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
                    draft: String::new(),
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
        self.broadcast_device_sync_message(&message);
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
                draft: String::new(),
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
            author_owner_pubkey_hex: None,
            author_picture_url: None,
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
        self.preferences
            .pinned_chat_ids
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
            Screen::DirectChatInfo { chat_id } => chat_id != &normalized,
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
        if kind == CHAT_MESSAGE_KIND {
            self.send_group_message(chat_id, content, unix_now(), None);
            return;
        }
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            return;
        };
        let Some(owner_pubkey) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return;
        };
        let created_at = unix_now();
        let mut nostr_tags = Vec::new();
        for tag in tags {
            if let Ok(tag) = nostr::Tag::parse(tag) {
                nostr_tags.push(tag);
            }
        }
        if let Ok(group_tag) = nostr::Tag::parse(["l", group_id.as_str()]) {
            nostr_tags.push(group_tag);
        }
        let mut unsigned = UnsignedEvent::new(
            owner_pubkey,
            Timestamp::from_secs(created_at.get()),
            Kind::Custom(kind as u16),
            nostr_tags,
            content.to_string(),
        );
        unsigned.ensure_id();
        let payload = match serde_json::to_vec(&unsigned) {
            Ok(payload) => payload,
            Err(error) => {
                self.push_debug_log("group.control.encode", error.to_string());
                return;
            }
        };
        let inner_event_id = unsigned.id.as_ref().map(ToString::to_string);
        let result = self
            .protocol_engine
            .as_mut()
            .map(|engine| engine.send_group_payload(&group_id, payload, inner_event_id.clone()));
        match result {
            Some(Ok(result)) => {
                self.process_protocol_engine_effects(result.effects);
                self.request_protocol_subscription_refresh();
            }
            Some(Err(error)) => self.push_debug_log("group.control.send", error.to_string()),
            None => {}
        }
        let _ = now_ms;
    }

    #[cfg(test)]
    pub(super) fn apply_decrypted_runtime_message(
        &mut self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        content: String,
        outer_event_id: Option<String>,
    ) {
        self.apply_decrypted_runtime_message_with_metadata(
            sender_owner,
            sender_device,
            None,
            content,
            outer_event_id,
        );
    }

    pub(super) fn apply_decrypted_runtime_message_with_metadata(
        &mut self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        conversation_owner: Option<PublicKey>,
        content: String,
        outer_event_id: Option<String>,
    ) {
        let is_supported_group_pairwise_payload =
            is_supported_group_pairwise_payload(content.as_bytes());
        let should_try_group_pairwise_payload = !looks_like_runtime_rumor(&content)
            || is_supported_group_pairwise_payload
            || self.is_local_sibling_group_runtime_payload(sender_owner, sender_device, &content);
        if should_try_group_pairwise_payload
            && self.try_apply_group_pairwise_payload(
                content.as_bytes(),
                sender_owner,
                sender_device,
                conversation_owner.is_some(),
                true,
            )
        {
            return;
        }

        let effective_sender_owner = self.direct_message_display_sender_owner(
            sender_owner,
            sender_device,
            conversation_owner,
        );

        let Some(runtime_rumor) = parse_runtime_rumor(&content) else {
            if looks_like_runtime_rumor(&content) {
                self.push_debug_log(
                    "runtime_rumor.decode.skip",
                    format!(
                        "sender_owner={} bytes={}",
                        effective_sender_owner.to_hex(),
                        content.len()
                    ),
                );
                return;
            }
            let chat_id = self.logged_in.as_ref().and_then(|logged_in| {
                direct_self_sync_chat_id(
                    effective_sender_owner,
                    logged_in.owner_pubkey,
                    conversation_owner,
                )
            });
            if !self
                .should_accept_direct_runtime_message(effective_sender_owner, chat_id.as_deref())
            {
                return;
            }
            self.apply_runtime_text_message(
                effective_sender_owner,
                chat_id,
                content,
                unix_now().get(),
                None,
                outer_event_id.clone(),
                outer_event_id,
            );
            return;
        };
        if !self.runtime_rumor_pubkey_matches_authenticated_sender(
            effective_sender_owner,
            sender_device,
            runtime_rumor.pubkey,
        ) {
            self.push_debug_log(
                "runtime_rumor.sender_mismatch",
                format!(
                    "sender_owner={} rumor_pubkey={}",
                    effective_sender_owner.to_hex(),
                    runtime_rumor.pubkey.to_hex()
                ),
            );
            return;
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
        let chat_id = chat_id_for_runtime_message(
            effective_sender_owner,
            local_owner,
            conversation_owner,
            runtime_rumor.tags.iter(),
        );
        let is_outgoing = effective_sender_owner == local_owner;
        if !is_outgoing
            && !is_group_chat_id(&chat_id)
            && !self.should_accept_direct_runtime_message(effective_sender_owner, Some(&chat_id))
        {
            return;
        }
        let inner_event_id = runtime_rumor.id.clone();
        self.acknowledge_delivered_group_runtime_rumor(
            &chat_id,
            effective_sender_owner,
            sender_device,
            created_at_secs,
        );

        match kind {
            CHAT_MESSAGE_KIND => {
                self.apply_runtime_text_message(
                    effective_sender_owner,
                    Some(chat_id.clone()),
                    runtime_rumor.content,
                    created_at_secs,
                    expires_at_secs,
                    Some(inner_event_id.clone()),
                    outer_event_id.clone(),
                );
                if !is_outgoing
                    && self.preferences.send_read_receipts
                    && !self.thread_is_message_request(&chat_id)
                {
                    // Suppress delivered receipts for unaccepted
                    // requests so the sender can't tell whether the
                    // user has seen the conversation arrive.
                    self.queue_delivered_receipt(&chat_id, inner_event_id);
                }
            }
            REACTION_KIND => {
                let sender_hex = effective_sender_owner.to_hex();
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
            TYPING_KIND if !is_outgoing => {
                self.apply_typing_event(
                    chat_id,
                    effective_sender_owner.to_hex(),
                    created_at_secs,
                    expires_at_secs,
                );
            }
            CHAT_SETTINGS_KIND => {
                let actor = self.owner_display_label(&effective_sender_owner.to_hex());
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

    fn try_apply_group_pairwise_payload(
        &mut self,
        payload: &[u8],
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        from_local_sibling: bool,
        log_errors: bool,
    ) -> bool {
        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        let group_outcome = match if from_local_sibling {
            protocol_engine.process_local_sibling_group_pairwise_payload(
                payload,
                sender_owner,
                sender_device,
            )
        } else {
            protocol_engine.process_group_pairwise_payload(payload, sender_owner, sender_device)
        } {
            Ok(group_outcome) => group_outcome,
            Err(error) => {
                if log_errors {
                    self.push_debug_log("appcore.protocol.group.payload.error", error.to_string());
                }
                return log_errors;
            }
        };
        if !group_outcome.consumed
            && group_outcome.events.is_empty()
            && group_outcome.effects.is_empty()
        {
            return false;
        }
        for group_event in group_outcome.events {
            self.apply_group_decrypted_event(group_event);
        }
        self.process_protocol_engine_effects(group_outcome.effects);
        self.request_protocol_subscription_refresh();
        self.schedule_fast_protocol_retry_if_pending();
        true
    }

    pub(super) fn acknowledge_delivered_group_runtime_rumor(
        &mut self,
        chat_id: &str,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        created_at_secs: u64,
    ) {
        let Some(group_id) = parse_group_id_from_chat_id(chat_id) else {
            return;
        };
        let acknowledged = self
            .protocol_engine
            .as_mut()
            .is_some_and(|protocol_engine| {
                protocol_engine.acknowledge_delivered_group_sender_key_message(
                    &group_id,
                    sender_owner,
                    sender_device,
                    created_at_secs,
                )
            });
        if acknowledged {
            self.push_debug_log(
                "appcore.protocol.sender_key.ack",
                format!(
                    "group_id={group_id} sender_owner={} created_at={created_at_secs}",
                    sender_owner.to_hex()
                ),
            );
        }
    }

    fn is_local_sibling_group_runtime_payload(
        &self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return false;
        };
        let from_local_device = sender_owner == local_owner
            || self.is_known_local_owner_device_pubkey(sender_owner)
            || sender_device.is_some_and(|device| self.is_known_local_owner_device_pubkey(device));
        if !from_local_device {
            return false;
        }
        parse_runtime_rumor(content)
            .is_some_and(|rumor| first_tag_value(rumor.tags.iter(), "l").is_some())
    }

    fn direct_message_display_sender_owner(
        &self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        conversation_owner: Option<PublicKey>,
    ) -> PublicKey {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return sender_owner;
        };
        if let Some(owner) = sender_device
            .and_then(|device| self.direct_owner_for_known_device_pubkey(device))
            .or_else(|| self.direct_owner_for_known_device_pubkey(sender_owner))
        {
            return owner;
        }
        if let Some(conversation_owner) = conversation_owner {
            if conversation_owner != logged_in.owner_pubkey
                && (sender_owner == logged_in.owner_pubkey
                    || self.is_known_local_owner_device_pubkey(sender_owner)
                    || sender_device
                        .is_some_and(|device| self.is_known_local_owner_device_pubkey(device)))
            {
                return logged_in.owner_pubkey;
            }
        }
        sender_owner
    }

    pub(super) fn runtime_rumor_pubkey_matches_authenticated_sender(
        &self,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        rumor_pubkey: PublicKey,
    ) -> bool {
        if rumor_pubkey == sender_owner {
            return true;
        }
        if sender_device.is_some_and(|device| rumor_pubkey == device) {
            return true;
        }
        self.direct_owner_for_known_device_pubkey(rumor_pubkey)
            .is_some_and(|owner| owner == sender_owner)
    }

    pub(super) fn should_accept_direct_runtime_message(
        &mut self,
        sender_owner: PublicKey,
        chat_id: Option<&str>,
    ) -> bool {
        let Some(local_owner) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey)
        else {
            return false;
        };
        let sender_hex = sender_owner.to_hex();
        // Blocked peers are dropped regardless of any other setting,
        // including self-sync rumors from a linked device that
        // happens to share a session with the blocked peer. This is
        // the local-ingest guard; the same blocklist also keeps the
        // peer out of the nostr + push subscriptions in the first
        // place, but events can still arrive via mirrored relays.
        if sender_owner != local_owner && self.is_owner_blocked(&sender_hex) {
            self.push_debug_log(
                "runtime_rumor.blocked_sender.skip",
                format!("sender_owner={sender_hex}"),
            );
            return false;
        }
        if sender_owner == local_owner || self.preferences.accept_unknown_direct_messages {
            return true;
        }
        let known = chat_id
            .and_then(|chat_id| self.threads.get(chat_id))
            .is_some()
            || self.threads.contains_key(&sender_hex);
        if !known {
            self.push_debug_log(
                "runtime_rumor.unknown_sender.skip",
                format!("sender_owner={sender_hex}"),
            );
        }
        known
    }

    fn direct_owner_for_known_device_pubkey(&self, device_pubkey: PublicKey) -> Option<PublicKey> {
        let device_hex = device_pubkey.to_hex();
        for known in self.app_keys.values() {
            if known
                .devices
                .iter()
                .any(|device| device.identity_pubkey_hex == device_hex)
            {
                return PublicKey::parse(&known.owner_pubkey_hex).ok();
            }
        }

        let hint = self
            .protocol_engine
            .as_ref()
            .and_then(|protocol_engine| protocol_engine.owner_hint_for_device(device_pubkey))?;
        if hint.verified || self.should_trust_claimed_direct_owner(hint.owner) {
            Some(hint.owner)
        } else {
            None
        }
    }

    fn should_trust_claimed_direct_owner(&self, owner: PublicKey) -> bool {
        let owner_hex = owner.to_hex();
        self.tracked_peer_owner_hexes().contains(&owner_hex)
            || self.owner_profiles.contains_key(&owner_hex)
    }

    fn is_known_local_owner_device_pubkey(&self, device_pubkey: PublicKey) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        if device_pubkey == logged_in.device_keys.public_key() {
            return true;
        }
        let owner_hex = logged_in.owner_pubkey.to_hex();
        if self.app_keys.get(&owner_hex).is_some_and(|app_keys| {
            app_keys
                .devices
                .iter()
                .any(|device| device.identity_pubkey_hex == device_pubkey.to_hex())
        }) {
            return true;
        }
        self.protocol_engine
            .as_ref()
            .is_some_and(|protocol_engine| {
                protocol_engine.is_known_local_owner_device(device_pubkey)
            })
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
            Some(sender_owner.to_hex()),
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
                .map(|group| {
                    group
                        .members
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
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
                display_name: self.owner_display_label(&owner_pubkey_hex),
                picture_url: self.owner_picture_url(&owner_pubkey_hex),
                owner_pubkey_hex,
                delivery: DeliveryState::Pending,
                updated_at_secs: created_at_secs,
            })
            .collect()
    }
}

pub(super) fn chat_message_from_persisted(message: &PersistedMessage) -> ChatMessageSnapshot {
    let (body, parsed_attachments) = extract_message_attachments(&message.body);
    ChatMessageSnapshot {
        id: message.id.clone(),
        chat_id: message.chat_id.clone(),
        kind: message.kind.clone(),
        author: message.author.clone(),
        author_owner_pubkey_hex: message.author_owner_pubkey_hex.clone(),
        author_picture_url: None,
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

pub(super) fn message_order(message: &ChatMessageSnapshot) -> u64 {
    message.created_at_secs
}
