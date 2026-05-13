use super::*;
use crate::core::projection::relay_connection_status;

const PROTOCOL_SUBSCRIPTION_ID: &str = "ndr-protocol";
const PROTOCOL_SUBSCRIPTION_APPLY_TIMEOUT_SECS: u64 = 8;
const PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS: u64 = 30;
pub(super) const PROTOCOL_RECONNECT_CHECK_SECS: u64 = 2;
const RELAY_TRANSPORT_RETRY_BACKOFF_SECS: [u64; 5] = [2, 5, 15, 30, 60];
#[cfg(not(test))]
const NEW_MESSAGE_AUTHOR_DELAYED_BACKFILL_MS: [u64; 2] = [2_500, 10_000];
#[cfg(test)]
const NEW_MESSAGE_AUTHOR_DELAYED_BACKFILL_MS: [u64; 1] = [50];

impl AppCore {
    pub(super) fn send_protocol_engine_unsigned_event(
        &mut self,
        peer: PublicKey,
        chat_id: &str,
        unsigned: UnsignedEvent,
        reason: &'static str,
    ) -> bool {
        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        match protocol_engine.send_direct_unsigned_event(peer, chat_id, unsigned, unix_now()) {
            Ok(result) => {
                self.push_debug_log(
                    "appcore.protocol.send",
                    format!(
                        "reason={reason} chat_id={chat_id} event_ids={} queued_targets={}",
                        result.event_ids.len(),
                        result.queued_targets.len()
                    ),
                );
                self.process_protocol_engine_effects_with_completions(
                    result.effects,
                    &BTreeMap::new(),
                );
                if !result.queued_targets.is_empty() {
                    self.handle_queued_protocol_targets(reason, &result.queued_targets);
                }
                true
            }
            Err(error) => {
                self.push_debug_log(
                    "appcore.protocol.send.error",
                    format!("reason={reason} chat_id={chat_id} error={error}"),
                );
                false
            }
        }
    }

    pub(super) fn process_protocol_engine_retry_batch(
        &mut self,
        reason: &'static str,
        batch: ProtocolRetryBatch,
    ) {
        if batch.direct_results.is_empty()
            && batch.group_result.events.is_empty()
            && batch.group_result.effects.is_empty()
            && batch.group_result.queued_targets.is_empty()
            && batch.direct_messages.is_empty()
            && batch.effects.is_empty()
        {
            return;
        }
        let mut published = 0usize;
        let mut queued_targets = Vec::new();
        let mut direct_effects = Vec::new();
        let mut direct_completions = BTreeMap::new();
        for result in batch.direct_results {
            published = published.saturating_add(result.event_ids.len());
            queued_targets.extend(result.queued_targets.clone());
            direct_completions.extend(
                result
                    .event_ids
                    .iter()
                    .map(|event_id| {
                        (
                            event_id.clone(),
                            (result.message_id.clone(), result.chat_id.clone()),
                        )
                    })
                    .collect::<BTreeMap<_, _>>(),
            );
            direct_effects.extend(result.effects);
            self.sync_message_delivery_trace(&result.chat_id, &result.message_id);
            self.reconcile_outgoing_message_delivery(&result.chat_id, &result.message_id);
        }
        queued_targets.extend(batch.group_result.queued_targets.clone());
        normalize_protocol_queued_targets(&mut queued_targets);
        self.process_protocol_engine_effects_with_completions(direct_effects, &direct_completions);
        for group_event in batch.group_result.events {
            self.apply_group_decrypted_event(group_event);
        }
        self.process_protocol_engine_effects_with_completions(
            batch.group_result.effects,
            &BTreeMap::new(),
        );
        self.process_protocol_engine_effects_with_completions(batch.effects, &BTreeMap::new());
        for decrypted in batch.direct_messages {
            let event_id = decrypted.event_id.clone();
            self.apply_decrypted_runtime_message_with_metadata(
                decrypted.sender,
                decrypted.sender_device,
                decrypted.conversation_owner,
                decrypted.content,
                decrypted.event_id,
            );
            if let Some(event_id) = event_id {
                self.remember_event(event_id);
            }
        }
        self.push_debug_log(
            "appcore.protocol.retry",
            format!(
                "reason={reason} published={published} queued_targets={}",
                queued_targets.len()
            ),
        );
        if queued_targets.is_empty() {
            self.request_protocol_subscription_refresh();
        } else {
            self.handle_queued_protocol_targets(reason, &queued_targets);
        }
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn handle_queued_protocol_targets(
        &mut self,
        reason: &'static str,
        queued_targets: &[String],
    ) {
        if queued_targets.is_empty() {
            return;
        }
        let mut queued_targets = queued_targets.to_vec();
        normalize_protocol_queued_targets(&mut queued_targets);
        if queued_targets.is_empty() {
            return;
        }
        self.push_debug_log(
            "appcore.protocol.queued",
            format!("reason={reason} targets={}", queued_targets.join(",")),
        );
        self.request_protocol_subscription_refresh();
        self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
            PROTOCOL_RECONNECT_CHECK_SECS,
        ));
    }

    pub(super) fn retry_protocol_engine_pending_outbound(&mut self, reason: &'static str) {
        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return;
        };
        let results = match protocol_engine.retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
        {
            Ok(results) => results,
            Err(error) => {
                self.push_debug_log("appcore.protocol.retry.error", error.to_string());
                return;
            }
        };
        self.process_protocol_engine_retry_batch(reason, results);
    }

    pub(super) fn append_protocol_retry_batch(
        target: &mut ProtocolRetryBatch,
        mut source: ProtocolRetryBatch,
    ) {
        target.direct_results.append(&mut source.direct_results);
        target
            .group_result
            .events
            .append(&mut source.group_result.events);
        target
            .group_result
            .effects
            .append(&mut source.group_result.effects);
        target
            .group_result
            .queued_targets
            .append(&mut source.group_result.queued_targets);
        target.direct_messages.append(&mut source.direct_messages);
        target.effects.append(&mut source.effects);
    }

    pub(super) fn remember_recent_handshake_peer(
        &mut self,
        owner_hex: String,
        device_hex: String,
        now_secs: u64,
    ) {
        if self.threads.contains_key(&owner_hex) {
            self.recent_handshake_peers
                .retain(|_, peer| peer.owner_hex != owner_hex);
            return;
        }
        self.recent_handshake_peers.insert(
            device_hex.clone(),
            RecentHandshakePeer {
                owner_hex,
                device_hex,
                observed_at_secs: now_secs,
            },
        );
    }

    pub(super) fn tracked_peer_owner_hexes(&self) -> HashSet<String> {
        let mut owners = self
            .threads
            .keys()
            .filter(|chat_id| !is_group_chat_id(chat_id))
            .cloned()
            .collect::<HashSet<_>>();
        if let Some(chat_id) = self.active_chat_id.as_ref() {
            if !is_group_chat_id(chat_id) {
                owners.insert(chat_id.clone());
            }
        }
        if let Some(logged_in) = self.logged_in.as_ref() {
            let local_owner_hex = logged_in.owner_pubkey.to_hex();
            for group in self.groups.values() {
                for member in &group.members {
                    let member = member.to_string();
                    if member != local_owner_hex {
                        owners.insert(member);
                    }
                }
            }
        }
        owners
    }

    pub(super) fn protocol_owner_hexes(&self) -> HashSet<String> {
        let mut owners = self.tracked_peer_owner_hexes();
        owners.extend(
            self.recent_handshake_peers
                .values()
                .map(|peer| peer.owner_hex.clone()),
        );
        owners.extend(self.app_keys.keys().cloned());
        if let Some(logged_in) = self.logged_in.as_ref() {
            owners.insert(logged_in.owner_pubkey.to_hex());
        }
        owners
    }

    pub(super) fn schedule_tracked_peer_catch_up(&self, after: Duration) {
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(after).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::FetchTrackedPeerCatchUp,
            )));
        });
    }

    pub(super) fn schedule_protocol_subscription_liveness_check(&mut self, after: Duration) {
        if !self.has_protocol_liveness_work() {
            self.protocol_subscription_runtime.liveness_due_at = None;
            return;
        }
        let due_at = Instant::now() + after;
        if self
            .protocol_subscription_runtime
            .liveness_due_at
            .is_some_and(|existing| existing <= due_at)
        {
            return;
        }
        self.protocol_subscription_runtime.liveness_due_at = Some(due_at);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        let token = self.protocol_reconnect_token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep_until(due_at).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::ProtocolSubscriptionLivenessCheck { token },
            )));
        });
    }

    fn has_protocol_liveness_work(&self) -> bool {
        self.protocol_subscription_runtime.desired_plan.is_some()
            || self.protocol_subscription_runtime.applying_plan.is_some()
            || self.protocol_subscription_runtime.applied_plan.is_some()
            || self.protocol_subscription_runtime.refresh_in_flight
            || self.protocol_subscription_runtime.refresh_dirty
            || !self.pending_relay_publishes.is_empty()
            || self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.has_pending_inbound_direct_events())
    }

    pub(super) fn schedule_pending_device_invite_poll(&mut self, after: Duration) {
        if !self.can_poll_pending_device_invites() {
            return;
        }
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        let token = self.device_invite_poll_token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(after).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::PollPendingDeviceInvites { token },
            )));
        });
    }

    pub(super) fn fetch_recent_messages_for_author(
        &self,
        author_pubkey: PublicKey,
        now: UnixSeconds,
        lookback_secs: u64,
    ) {
        self.fetch_recent_messages_for_authors(vec![author_pubkey], now, lookback_secs);
    }

    fn fetch_recent_messages_for_authors(
        &self,
        author_pubkeys: Vec<PublicKey>,
        now: UnixSeconds,
        lookback_secs: u64,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if author_pubkeys.is_empty() {
            return;
        }
        let filter = build_direct_message_backfill_filter(
            author_pubkeys,
            now.get().saturating_sub(lookback_secs),
            DEVICE_INVITE_DISCOVERY_LIMIT,
        );
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(5)).await;
            if let Ok(events) = client.fetch_events(filter, Duration::from_secs(5)).await {
                let collected = events.iter().cloned().collect::<Vec<_>>();
                if !collected.is_empty() {
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::FetchCatchUpEvents(collected),
                    )));
                }
            }
        });
    }

    fn schedule_new_message_author_backfill(
        &self,
        author_pubkeys: Vec<PublicKey>,
        lookback_secs: u64,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if author_pubkeys.is_empty() {
            return;
        }
        for delay_ms in NEW_MESSAGE_AUTHOR_DELAYED_BACKFILL_MS {
            let client = client.clone();
            let relay_urls = relay_urls.clone();
            let authors = author_pubkeys.clone();
            let tx = self.core_sender.clone();
            self.runtime.spawn(async move {
                sleep(Duration::from_millis(delay_ms)).await;
                let filter = build_direct_message_backfill_filter(
                    authors,
                    unix_now().get().saturating_sub(lookback_secs),
                    DEVICE_INVITE_DISCOVERY_LIMIT,
                );
                ensure_session_relays_configured(&client, &relay_urls).await;
                connect_client_with_timeout(&client, Duration::from_secs(5)).await;
                if let Ok(events) = client.fetch_events(filter, Duration::from_secs(5)).await {
                    let collected = events.iter().cloned().collect::<Vec<_>>();
                    if !collected.is_empty() {
                        let _ = tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::FetchCatchUpEvents(collected),
                        )));
                    }
                }
            });
        }
    }

    pub(super) fn fetch_recent_group_sender_key_messages_for_author(
        &self,
        author_pubkey: PublicKey,
        now: UnixSeconds,
        lookback_secs: u64,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        let filter = Filter::new()
            .kind(Kind::from(GROUP_SENDER_KEY_MESSAGE_KIND as u16))
            .authors(vec![author_pubkey])
            .since(Timestamp::from(now.get().saturating_sub(lookback_secs)))
            .limit(DEVICE_INVITE_DISCOVERY_LIMIT);
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(5)).await;
            if let Ok(events) = client.fetch_events(filter, Duration::from_secs(5)).await {
                let collected = events.iter().cloned().collect::<Vec<_>>();
                if !collected.is_empty() {
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::FetchCatchUpEvents(collected),
                    )));
                }
            }
        });
    }

    pub(super) fn fetch_recent_messages_for_tracked_peers(&self, now: UnixSeconds) {
        let direct_authors = self
            .protocol_subscription_runtime
            .desired_plan
            .as_ref()
            .map(|plan| {
                plan.message_authors
                    .iter()
                    .filter_map(|hex| PublicKey::parse(hex).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                self.protocol_engine
                    .as_ref()
                    .map(ProtocolEngine::known_message_author_pubkeys)
                    .unwrap_or_default()
            });
        let group_authors = self
            .protocol_subscription_runtime
            .desired_plan
            .as_ref()
            .map(|plan| {
                plan.group_sender_key_authors
                    .iter()
                    .filter_map(|hex| PublicKey::parse(hex).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                self.protocol_engine
                    .as_ref()
                    .map(ProtocolEngine::known_group_sender_event_pubkeys)
                    .unwrap_or_default()
            });
        for author in direct_authors {
            self.fetch_recent_messages_for_author(author, now, CATCH_UP_LOOKBACK_SECS);
        }
        for author in group_authors {
            self.fetch_recent_group_sender_key_messages_for_author(
                author,
                now,
                CATCH_UP_LOOKBACK_SECS,
            );
        }
    }

    pub(super) fn recent_protocol_filters(&self, now: UnixSeconds) -> Vec<Filter> {
        let Some(plan) = self
            .protocol_subscription_runtime
            .desired_plan
            .clone()
            .or_else(|| self.compute_protocol_subscription_plan())
        else {
            return Vec::new();
        };
        let owners = pubkeys_from_hexes(&plan.roster_authors);
        let mut filters = if owners.is_empty() {
            Vec::new()
        } else {
            vec![Filter::new().kind(Kind::Metadata).authors(owners.clone())]
        };
        if !owners.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                    .authors(owners.clone())
                    .identifier(NDR_APP_KEYS_D_TAG)
                    .since(Timestamp::from(
                        now.get()
                            .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                    ))
                    .limit(DEVICE_INVITE_DISCOVERY_LIMIT),
            );
        }
        let invite_authors = pubkeys_from_hexes(&plan.invite_authors);
        if !invite_authors.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(INVITE_EVENT_KIND as u16))
                    .authors(invite_authors.clone())
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::L), NDR_INVITES_L_TAG)
                    .since(Timestamp::from(
                        now.get()
                            .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                    ))
                    .limit(DEVICE_INVITE_DISCOVERY_LIMIT),
            );
        }
        let message_authors = pubkeys_from_hexes(&plan.message_authors);
        if !message_authors.is_empty() {
            filters.push(build_direct_message_backfill_filter(
                message_authors,
                now.get().saturating_sub(CATCH_UP_LOOKBACK_SECS),
                DEVICE_INVITE_DISCOVERY_LIMIT,
            ));
        }

        let group_sender_key_authors = pubkeys_from_hexes(&plan.group_sender_key_authors);
        if !group_sender_key_authors.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(GROUP_SENDER_KEY_MESSAGE_KIND as u16))
                    .authors(group_sender_key_authors)
                    .since(Timestamp::from(
                        now.get().saturating_sub(CATCH_UP_LOOKBACK_SECS),
                    ))
                    .limit(DEVICE_INVITE_DISCOVERY_LIMIT),
            );
        }

        let private_invite_response_pubkeys = plan
            .invite_response_recipient
            .as_deref()
            .map(pubkeys_from_comma_separated_hexes)
            .unwrap_or_default();
        if !private_invite_response_pubkeys.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                    .pubkeys(private_invite_response_pubkeys)
                    .since(Timestamp::from(
                        now.get()
                            .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                    )),
            );
        }
        if !invite_authors.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                    .authors(invite_authors)
                    .since(Timestamp::from(
                        now.get()
                            .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                    ))
                    .limit(DEVICE_INVITE_DISCOVERY_LIMIT),
            );
        }
        filters
    }

    fn protocol_invite_author_pubkeys(&self, owners: &[PublicKey]) -> Vec<PublicKey> {
        let mut authors = Vec::new();
        for owner in owners {
            if let Some(known) = self.app_keys.get(&owner.to_hex()) {
                for device in &known.devices {
                    if let Ok(pubkey) = PublicKey::parse(&device.identity_pubkey_hex) {
                        authors.push(pubkey);
                    }
                }
            }
            if let Some(protocol_engine) = self.protocol_engine.as_ref() {
                authors.extend(protocol_engine.known_device_identity_pubkeys_for_owner(*owner));
            }
        }
        authors.sort_by_key(|pubkey| pubkey.to_hex());
        authors.dedup();
        authors
    }

    fn protocol_invite_response_pubkeys(&self) -> Vec<PublicKey> {
        let mut pubkeys = self.private_chat_invite_response_pubkeys();
        if let Some(local_invite_pubkey) = self.logged_in.as_ref().and_then(|logged_in| {
            logged_in
                .local_invite
                .inviter_ephemeral_public_key
                .to_nostr()
                .ok()
        }) {
            pubkeys.push(local_invite_pubkey);
        }
        pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
        pubkeys.dedup();
        pubkeys
    }

    pub(super) fn fetch_recent_protocol_state(&mut self) -> bool {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .filter(|logged_in| !logged_in.relay_urls.is_empty())
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return false;
        };
        let now = unix_now();
        let filters = self.recent_protocol_filters(now);
        if filters.is_empty() {
            return false;
        }
        self.push_debug_log(
            "protocol.catch_up.fetch",
            format!("filters={}", filters.len()),
        );
        self.state.busy.syncing_network = true;

        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(5)).await;
            match fetch_events_for_filters(&client, filters, Duration::from_secs(5)).await {
                Ok(collected) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "protocol.catch_up.result".to_string(),
                        detail: format!("events={}", collected.len()),
                    })));
                    if !collected.is_empty() {
                        let _ = tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::FetchCatchUpEvents(collected),
                        )));
                    }
                }
                Err(error) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "protocol.catch_up.error".to_string(),
                        detail: error.to_string(),
                    })));
                }
            }
            let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::SyncComplete)));
        });
        true
    }

    pub(super) fn fetch_protocol_state_for_filters(
        &mut self,
        filters: Vec<Filter>,
        reason: &'static str,
    ) -> bool {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .filter(|logged_in| !logged_in.relay_urls.is_empty())
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return false;
        };
        if filters.is_empty() {
            return false;
        }
        self.push_debug_log(
            "protocol.engine_fetch.fetch",
            format!("reason={reason} filters={}", filters.len()),
        );
        self.state.busy.syncing_network = true;

        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(5)).await;
            match fetch_events_for_filters(&client, filters, Duration::from_secs(5)).await {
                Ok(collected) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "protocol.engine_fetch.result".to_string(),
                        detail: format!("events={}", collected.len()),
                    })));
                    if !collected.is_empty() {
                        let _ = tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::FetchCatchUpEvents(collected),
                        )));
                    }
                }
                Err(error) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "protocol.engine_fetch.error".to_string(),
                        detail: error.to_string(),
                    })));
                }
            }
            let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::SyncComplete)));
        });
        true
    }

    pub(super) fn fetch_pending_device_invites_for_local_owner(&mut self) {
        self.fetch_recent_protocol_state();
    }

    fn tracked_peer_protocol_backfill_needed(&self) -> bool {
        let tracked_peer_owners = self.tracked_peer_owner_hexes();
        if tracked_peer_owners.is_empty() {
            return false;
        }

        tracked_peer_owners
            .iter()
            .any(|owner_hex| !self.app_keys.contains_key(owner_hex))
            || self.protocol_engine.as_ref().is_some_and(|engine| {
                tracked_peer_owners.iter().any(|owner_hex| {
                    PublicKey::parse(owner_hex).is_ok_and(|owner_pubkey| {
                        let owner_prefix = owner_pubkey.to_hex();
                        engine
                            .queued_message_diagnostics(None)
                            .iter()
                            .any(|target| target == &owner_prefix)
                    })
                })
            })
            || self.protocol_engine.as_ref().is_some_and(|engine| {
                tracked_peer_owners.iter().any(|owner_hex| {
                    PublicKey::parse(owner_hex).is_ok_and(|owner_pubkey| {
                        engine
                            .message_author_pubkeys_for_owner(owner_pubkey)
                            .is_empty()
                    })
                })
            })
    }

    pub(super) fn current_queued_protocol_targets(&self) -> Vec<String> {
        let mut targets = Vec::new();
        if let Some(protocol_engine) = self.protocol_engine.as_ref() {
            targets.extend(protocol_engine.queued_message_diagnostics(None));
            targets.extend(protocol_engine.queued_owner_claim_targets());
            targets.extend(
                protocol_engine
                    .debug_snapshot()
                    .pending_group_fanout_targets,
            );
        }
        targets.sort();
        targets.dedup();
        targets
    }

    pub(super) fn start_notifications_loop(&self, client: Client) {
        let mut notifications = client.notifications();
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            loop {
                match notifications.recv().await {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
                            (*event).clone(),
                        ))));
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub(super) fn start_relay_status_watchers(&mut self) {
        let Some(client) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.client.clone())
        else {
            return;
        };
        let relays = self.runtime.block_on(client.relays());
        for (relay_url, relay) in relays {
            let relay_url = normalize_nostr_relay_url(&relay_url.to_string())
                .unwrap_or_else(|_| relay_url.to_string());
            if !self.relay_status_watch_urls.insert(relay_url.clone()) {
                continue;
            }
            let generation = self.relay_status_watch_generation;
            let mut notifications = relay.notifications();
            let tx = self.core_sender.clone();
            self.runtime.spawn(async move {
                loop {
                    match notifications.recv().await {
                        Ok(RelayNotification::RelayStatus { status }) => {
                            let _ = tx.send(CoreMsg::Internal(Box::new(
                                InternalEvent::RelayStatusChanged {
                                    relay_url: relay_url.clone(),
                                    status,
                                    generation,
                                },
                            )));
                        }
                        Ok(RelayNotification::Shutdown) => {
                            let _ = tx.send(CoreMsg::Internal(Box::new(
                                InternalEvent::RelayStatusChanged {
                                    relay_url: relay_url.clone(),
                                    status: RelayStatus::Terminated,
                                    generation,
                                },
                            )));
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
    }

    pub(super) fn schedule_session_connect(&mut self) {
        self.request_relay_connection("session_connect", false);
    }

    pub(super) fn request_relay_connection(
        &mut self,
        reason: impl Into<String>,
        force_reconnect: bool,
    ) {
        let reason = reason.into();
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if relay_urls.is_empty() {
            return;
        }
        if self.relay_transport_runtime.connect_in_flight {
            self.relay_transport_runtime.connect_dirty = true;
            self.relay_transport_runtime.force_reconnect_dirty |= force_reconnect;
            self.push_debug_log(
                "relay.transport.connect",
                format!("reason={reason} deferred=in_flight force_reconnect={force_reconnect}"),
            );
            return;
        }

        self.relay_transport_runtime.connect_in_flight = true;
        self.relay_transport_runtime.connect_dirty = false;
        self.relay_transport_runtime.force_reconnect_dirty = false;
        self.relay_transport_runtime.connect_token =
            self.relay_transport_runtime.connect_token.wrapping_add(1);
        self.relay_transport_runtime.last_connect_reason = Some(reason.clone());
        let token = self.relay_transport_runtime.connect_token;
        let tx = self.core_sender.clone();
        self.push_debug_log(
            "relay.transport.connect",
            format!("reason={reason} start force_reconnect={force_reconnect}"),
        );
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            if force_reconnect {
                let _ = client.disconnect().await;
            }
            connect_client_with_timeout(&client, Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS))
                .await;
            let connected_count =
                wait_for_connected_relays(&client, Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS))
                    .await as u64;
            let relay_statuses = client
                .relays()
                .await
                .into_iter()
                .map(|(relay_url, relay)| {
                    let relay_url = normalize_nostr_relay_url(&relay_url.to_string())
                        .unwrap_or_else(|_| relay_url.to_string());
                    (relay_url, relay.status())
                })
                .collect::<Vec<_>>();
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::RelayTransportConnectionFinished {
                    token,
                    reason,
                    relay_statuses,
                    connected_count,
                },
            )));
        });
    }

    pub(super) fn handle_relay_transport_connection_finished(
        &mut self,
        token: u64,
        reason: String,
        relay_statuses: Vec<(String, RelayStatus)>,
        connected_count: u64,
    ) {
        if token != self.relay_transport_runtime.connect_token {
            return;
        }
        self.relay_transport_runtime.connect_in_flight = false;
        let connect_dirty = self.relay_transport_runtime.connect_dirty;
        let force_reconnect_dirty = self.relay_transport_runtime.force_reconnect_dirty;
        self.relay_transport_runtime.connect_dirty = false;
        self.relay_transport_runtime.force_reconnect_dirty = false;
        self.apply_relay_statuses(relay_statuses);
        self.push_debug_log(
            "relay.transport.connect",
            format!(
                "reason={reason} connected={} cached_connected={} dirty={connect_dirty}",
                connected_count, self.relay_connected_count
            ),
        );

        if self.relay_connected_count > 0 || connected_count > 0 {
            self.relay_transport_runtime.retry_backoff_attempt = 0;
            self.relay_transport_runtime.next_retry_due_at = None;
            self.relay_transport_runtime.next_retry_reason = None;
            self.reconcile_protocol_subscriptions("relay_transport_connected", false);
            self.retry_protocol_engine_pending_outbound("relay_transport_connected");
            self.retry_pending_relay_publishes("relay_transport_connected");
        } else {
            self.schedule_relay_transport_retry("connect_failed");
        }

        if connect_dirty {
            self.request_relay_connection("coalesced_connect", force_reconnect_dirty);
        }
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn schedule_relay_transport_retry(&mut self, reason: impl Into<String>) {
        if self.logged_in.is_none() {
            return;
        }
        let reason = reason.into();
        let attempt_index =
            self.relay_transport_runtime
                .retry_backoff_attempt
                .min((RELAY_TRANSPORT_RETRY_BACKOFF_SECS.len() - 1) as u32) as usize;
        let delay = Duration::from_secs(RELAY_TRANSPORT_RETRY_BACKOFF_SECS[attempt_index]);
        self.relay_transport_runtime.retry_backoff_attempt = self
            .relay_transport_runtime
            .retry_backoff_attempt
            .saturating_add(1);
        self.relay_transport_runtime.next_retry_due_at = Some(Instant::now() + delay);
        self.relay_transport_runtime.next_retry_reason = Some(reason.clone());
        self.push_debug_log(
            "relay.transport.connect",
            format!("reason={reason} scheduled_retry_ms={}", delay.as_millis()),
        );
        self.schedule_protocol_subscription_liveness_check(delay);
    }

    pub(super) fn request_protocol_subscription_refresh(&mut self) {
        self.request_protocol_subscription_refresh_inner(false, false);
    }

    pub(super) fn request_protocol_subscription_refresh_forced(&mut self) {
        self.request_protocol_subscription_refresh_inner(true, false);
    }

    pub(super) fn request_protocol_subscription_refresh_forced_reconnect_if_offline(&mut self) {
        self.request_protocol_subscription_refresh_inner(true, true);
    }

    pub(super) fn request_protocol_subscription_refresh_inner(
        &mut self,
        force: bool,
        force_reconnect_if_offline: bool,
    ) {
        if self.logged_in.is_none() {
            self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
            self.relay_transport_runtime = RelayTransportRuntime::default();
            return;
        }
        if self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.relay_urls.is_empty())
            .unwrap_or(true)
        {
            self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
            self.relay_transport_runtime = RelayTransportRuntime::default();
            return;
        }

        let previous_desired = self.protocol_subscription_runtime.desired_plan.clone();
        let desired_plan = self.compute_protocol_subscription_plan();
        let plan_changed = previous_desired != desired_plan;
        self.note_protocol_plan_author_changes(previous_desired.as_ref(), desired_plan.as_ref());
        self.protocol_subscription_runtime.desired_plan = desired_plan.clone();

        let plan_summary = summarize_protocol_plan(desired_plan.as_ref());
        if force || plan_changed {
            self.protocol_subscription_runtime.refresh_token = self
                .protocol_subscription_runtime
                .refresh_token
                .wrapping_add(1);
            self.push_debug_log("protocol.subscription.refresh", plan_summary);
        }
        let unapplied = self.protocol_subscription_runtime.applied_plan != desired_plan;
        if force || plan_changed || unapplied {
            let reason = if force {
                "forced_refresh"
            } else {
                "plan_changed"
            };
            self.reconcile_protocol_subscriptions(reason, force_reconnect_if_offline);
        }
        self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
            PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS,
        ));
    }

    pub(super) fn compute_protocol_subscription_plan(&self) -> Option<ProtocolSubscriptionPlan> {
        let roster_authors = self
            .protocol_owner_hexes()
            .into_iter()
            .collect::<HashSet<_>>();
        let roster_authors = sorted_hexes(roster_authors);
        let protocol_owners = roster_authors
            .iter()
            .filter_map(|hex| PublicKey::parse(hex).ok())
            .collect::<Vec<_>>();
        let invite_authors = self
            .protocol_invite_author_pubkeys(&protocol_owners)
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<HashSet<_>>();
        let invite_authors = sorted_hexes(invite_authors);
        let message_authors = self
            .protocol_engine
            .as_ref()
            .map(ProtocolEngine::known_message_author_pubkeys)
            .unwrap_or_default()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<HashSet<_>>();
        let message_authors = sorted_hexes(message_authors);
        let group_sender_key_authors = self
            .protocol_engine
            .as_ref()
            .map(ProtocolEngine::known_group_sender_event_pubkeys)
            .unwrap_or_default()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<HashSet<_>>();
        let group_sender_key_authors = sorted_hexes(group_sender_key_authors);
        let invite_response_recipient = self
            .protocol_invite_response_pubkeys()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<Vec<_>>()
            .join(",");
        let invite_response_recipient =
            (!invite_response_recipient.is_empty()).then_some(invite_response_recipient);
        let has_filters = !roster_authors.is_empty()
            || !invite_authors.is_empty()
            || !message_authors.is_empty()
            || !group_sender_key_authors.is_empty()
            || invite_response_recipient.is_some();
        has_filters.then_some(ProtocolSubscriptionPlan {
            runtime_subscriptions: vec![PROTOCOL_SUBSCRIPTION_ID.to_string()],
            roster_authors,
            invite_authors,
            message_authors,
            group_sender_key_authors,
            invite_response_recipient,
        })
    }

    fn note_protocol_plan_author_changes(
        &mut self,
        previous: Option<&ProtocolSubscriptionPlan>,
        next: Option<&ProtocolSubscriptionPlan>,
    ) {
        let previous_message_authors = previous
            .map(|plan| plan.message_authors.iter().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();
        let next_message_authors = next
            .map(|plan| plan.message_authors.iter().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();
        let mut added_message_authors = next_message_authors
            .difference(&previous_message_authors)
            .cloned()
            .collect::<Vec<_>>();
        added_message_authors.sort();
        if !added_message_authors.is_empty() {
            self.mark_mobile_push_dirty();
        }
        let added_message_author_pubkeys = added_message_authors
            .into_iter()
            .filter_map(|author_hex| PublicKey::parse(&author_hex).ok())
            .collect::<Vec<_>>();
        if !added_message_author_pubkeys.is_empty() {
            self.fetch_recent_messages_for_authors(
                added_message_author_pubkeys.clone(),
                unix_now(),
                NEW_MESSAGE_AUTHOR_BACKFILL_LOOKBACK_SECS,
            );
            self.schedule_new_message_author_backfill(
                added_message_author_pubkeys,
                NEW_MESSAGE_AUTHOR_BACKFILL_LOOKBACK_SECS,
            );
        }

        let previous_group_authors = previous
            .map(|plan| {
                plan.group_sender_key_authors
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let next_group_authors = next
            .map(|plan| {
                plan.group_sender_key_authors
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let mut added_group_authors = next_group_authors
            .difference(&previous_group_authors)
            .cloned()
            .collect::<Vec<_>>();
        added_group_authors.sort();
        for author_hex in added_group_authors {
            if let Ok(author) = PublicKey::parse(&author_hex) {
                self.fetch_recent_group_sender_key_messages_for_author(
                    author,
                    unix_now(),
                    CATCH_UP_LOOKBACK_SECS,
                );
            }
        }
    }

    #[cfg(test)]
    pub(super) fn handle_relay_status_changed(&mut self, relay_url: String, status: RelayStatus) {
        self.handle_relay_status_changed_for_generation(
            relay_url,
            status,
            self.relay_status_watch_generation,
        );
    }

    pub(super) fn handle_relay_status_changed_for_generation(
        &mut self,
        relay_url: String,
        status: RelayStatus,
        generation: u64,
    ) {
        if generation != self.relay_status_watch_generation {
            return;
        }
        let normalized_relay_url =
            normalize_nostr_relay_url(&relay_url).unwrap_or_else(|_| relay_url.clone());
        if !self
            .configured_relay_url_set()
            .contains(&normalized_relay_url)
        {
            return;
        }
        let previous_status = self.relay_status_by_url.get(&normalized_relay_url).cloned();
        if previous_status
            .as_ref()
            .is_some_and(|existing| *existing == status)
        {
            return;
        }
        let previous_visible_status = previous_status
            .clone()
            .map(relay_connection_status)
            .unwrap_or("offline");
        let next_visible_status = relay_connection_status(status.clone());
        let was_connected = self.relay_connected_count > 0;
        self.relay_status_by_url
            .insert(normalized_relay_url.clone(), status.clone());
        self.refresh_relay_connection_status_from_cached_statuses();
        let is_connected = self.relay_connected_count > 0;
        let visible_status_changed = previous_visible_status != next_visible_status;
        if !visible_status_changed && was_connected == is_connected {
            return;
        }
        self.push_debug_log(
            "relay.status",
            format!("url={normalized_relay_url} status={status}"),
        );
        match status {
            RelayStatus::Connected if !was_connected && is_connected => {
                self.reconcile_protocol_subscriptions("relay_connected", false);
                self.fetch_recent_protocol_state();
                self.fetch_recent_messages_for_tracked_peers(unix_now());
                self.retry_protocol_engine_pending_outbound("relay_connected");
                self.retry_pending_relay_publishes("relay_connected");
                self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                    PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS,
                ));
            }
            RelayStatus::Connected => {}
            RelayStatus::Disconnected | RelayStatus::Terminated | RelayStatus::Sleeping
                if was_connected && !is_connected =>
            {
                self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                    PROTOCOL_RECONNECT_CHECK_SECS,
                ));
            }
            RelayStatus::Disconnected | RelayStatus::Terminated | RelayStatus::Sleeping => {}
            RelayStatus::Initialized
            | RelayStatus::Pending
            | RelayStatus::Connecting
            | RelayStatus::Banned => {}
        }
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn refresh_relay_connection_status(&mut self) {
        let relay_statuses = self.current_client_relay_statuses();
        self.apply_relay_statuses(relay_statuses);
    }

    pub(super) fn refresh_relay_connection_status_from_cached_statuses(&mut self) {
        let configured = self.configured_relay_url_set();
        let configured_relay_count = configured.len();
        self.relay_connected_count = self
            .relay_status_by_url
            .iter()
            .filter(|(url, status)| {
                configured.contains(url.as_str()) && **status == RelayStatus::Connected
            })
            .count() as u64;
        self.update_all_relays_offline_since(configured_relay_count);
    }

    fn apply_relay_statuses(&mut self, relay_statuses: Vec<(String, RelayStatus)>) {
        for (relay_url, status) in relay_statuses {
            self.relay_status_by_url.insert(relay_url, status);
        }
        self.refresh_relay_connection_status_from_cached_statuses();
    }

    fn update_all_relays_offline_since(&mut self, configured_relay_count: usize) {
        if configured_relay_count == 0 || self.relay_connected_count > 0 {
            self.all_relays_offline_since_secs = None;
        } else if self.all_relays_offline_since_secs.is_none() {
            self.all_relays_offline_since_secs = Some(unix_now().get());
        }
    }

    fn current_client_relay_statuses(&self) -> Vec<(String, RelayStatus)> {
        self.logged_in
            .as_ref()
            .map(|logged_in| {
                self.runtime.block_on(async {
                    logged_in
                        .client
                        .relays()
                        .await
                        .into_iter()
                        .map(|(relay_url, relay)| {
                            let relay_url = normalize_nostr_relay_url(&relay_url.to_string())
                                .unwrap_or_else(|_| relay_url.to_string());
                            (relay_url, relay.status())
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default()
    }

    fn configured_relay_url_set(&self) -> HashSet<String> {
        let mut configured = self
            .preferences
            .nostr_relay_urls
            .iter()
            .filter_map(|url| normalize_nostr_relay_url(url).ok())
            .collect::<HashSet<_>>();
        if let Some(logged_in) = self.logged_in.as_ref() {
            configured.extend(
                logged_in
                    .relay_urls
                    .iter()
                    .filter_map(|url| normalize_nostr_relay_url(&url.to_string()).ok()),
            );
        }
        configured
    }

    pub(super) fn handle_protocol_subscription_liveness_check(&mut self, token: u64) {
        if token != self.protocol_reconnect_token {
            return;
        }
        self.protocol_subscription_runtime.liveness_due_at = None;
        if self.logged_in.is_none() {
            return;
        }
        let has_subscription_work = self.protocol_subscription_runtime.desired_plan.is_some()
            || self.protocol_subscription_runtime.applied_plan.is_some()
            || self.protocol_subscription_runtime.applying_plan.is_some()
            || self.protocol_subscription_runtime.refresh_in_flight
            || self.protocol_subscription_runtime.refresh_dirty;
        let has_pending_relay_publishes = !self.pending_relay_publishes.is_empty();
        if !has_subscription_work && !has_pending_relay_publishes {
            return;
        }
        let connected_relays = self
            .logged_in
            .as_ref()
            .map(|logged_in| {
                self.runtime.block_on(async {
                    logged_in
                        .client
                        .relays()
                        .await
                        .values()
                        .filter(|relay| relay.status() == RelayStatus::Connected)
                        .count()
                })
            })
            .unwrap_or(0);
        let desired_unapplied = self.protocol_subscription_runtime.desired_plan
            != self.protocol_subscription_runtime.applied_plan;
        let should_retry_backfill = self.protocol_subscription_runtime.desired_plan.is_some()
            && (connected_relays == 0
                || self.tracked_peer_protocol_backfill_needed()
                || self.protocol_subscription_runtime.refresh_in_flight
                || self.protocol_subscription_runtime.refresh_dirty
                || desired_unapplied
                || self
                    .protocol_engine
                    .as_ref()
                    .is_some_and(|engine| engine.has_pending_inbound_direct_events()));
        self.push_debug_log(
            "protocol.liveness",
            format!(
                "connected={connected_relays} retry_backfill={should_retry_backfill} pending_publishes={}",
                self.pending_relay_publishes.len()
            ),
        );
        if has_subscription_work {
            self.reconcile_protocol_subscriptions("liveness_check", true);
        }
        if should_retry_backfill {
            let queued_targets = self.current_queued_protocol_targets();
            if queued_targets.is_empty() {
                self.fetch_recent_protocol_state();
            }
            self.fetch_recent_messages_for_tracked_peers(unix_now());
            self.retry_protocol_engine_pending_outbound("liveness_check");
        }
        if has_pending_relay_publishes {
            self.retry_pending_relay_publishes("liveness_check");
        }
        self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
            PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS,
        ));
    }

    pub(super) fn reconcile_protocol_subscriptions(
        &mut self,
        reason: &'static str,
        force_reconnect_if_offline: bool,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        let desired_plan = self.protocol_subscription_runtime.desired_plan.clone();
        if desired_plan.is_none()
            && self.protocol_subscription_runtime.applied_plan.is_none()
            && !self.protocol_subscription_runtime.refresh_in_flight
        {
            return;
        }
        if self.protocol_subscription_runtime.refresh_in_flight {
            self.protocol_subscription_runtime.refresh_dirty = true;
            self.protocol_subscription_runtime.force_reconnect_dirty |= force_reconnect_if_offline;
            self.push_debug_log(
                "protocol.subscription.reconcile",
                format!("reason={reason} deferred=in_flight"),
            );
            return;
        }
        self.refresh_relay_connection_status();
        if self.relay_connected_count == 0 {
            self.protocol_subscription_runtime.refresh_dirty = true;
            self.protocol_subscription_runtime.force_reconnect_dirty |= force_reconnect_if_offline;
            self.push_debug_log(
                "protocol.subscription.reconcile",
                format!("reason={reason} deferred=relay_offline"),
            );
            self.request_relay_connection(
                format!("subscription_reconcile:{reason}"),
                force_reconnect_if_offline && self.relay_connected_count > 0,
            );
            self.schedule_relay_transport_retry("subscription_reconcile_offline");
            return;
        }
        let previous_applied_plan = self.protocol_subscription_runtime.applied_plan.clone();
        let filters = desired_plan
            .as_ref()
            .map(build_protocol_subscription_filters)
            .unwrap_or_default();
        let filter_count = filters.len() as u64;
        self.protocol_subscription_runtime.refresh_in_flight = true;
        self.protocol_subscription_runtime.refresh_dirty = false;
        self.protocol_subscription_runtime.force_reconnect_dirty = false;
        self.protocol_subscription_runtime.applying_plan = desired_plan.clone();
        self.protocol_subscription_runtime.reconcile_token = self
            .protocol_subscription_runtime
            .reconcile_token
            .wrapping_add(1);
        let token = self.protocol_subscription_runtime.reconcile_token;
        let generation = self.protocol_reconnect_token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            let apply_result = tokio::time::timeout(
                Duration::from_secs(PROTOCOL_SUBSCRIPTION_APPLY_TIMEOUT_SECS),
                async {
                    ensure_session_relays_configured(&client, &relay_urls).await;
                    let connected_before = connected_relay_count_for_client(&client).await as u64;
                    if connected_before == 0 {
                        return ProtocolSubscriptionApplyOutput {
                            connected_before,
                            connected_after: connected_before,
                            filter_count: filters.len() as u64,
                            success: false,
                            error: Some("no connected relays".to_string()),
                        };
                    }
                    let subscription_id = SubscriptionId::new(PROTOCOL_SUBSCRIPTION_ID);
                    if previous_applied_plan.is_some() {
                        let _ = client.unsubscribe(&subscription_id).await;
                    }
                    let result = if filters.is_empty() {
                        Ok(())
                    } else {
                        subscribe_protocol_filters_with_id(&client, subscription_id, filters).await
                    };
                    let connected_after = connected_relay_count_for_client(&client).await as u64;
                    ProtocolSubscriptionApplyOutput {
                        connected_before,
                        connected_after,
                        filter_count,
                        success: result.is_ok(),
                        error: result.err(),
                    }
                },
            )
            .await;
            let mut output = match apply_result {
                Ok(output) => output,
                Err(_) => ProtocolSubscriptionApplyOutput {
                    connected_before: connected_relay_count_for_client(&client).await as u64,
                    connected_after: connected_relay_count_for_client(&client).await as u64,
                    filter_count,
                    success: false,
                    error: Some("timed out".to_string()),
                },
            };
            if output.connected_after == 0 {
                output.connected_after = connected_relay_count_for_client(&client).await as u64;
            }
            let relay_statuses = current_client_relay_statuses(&client).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::ProtocolSubscriptionReconcileCompleted {
                    generation,
                    token,
                    reason: reason.to_string(),
                    plan: desired_plan,
                    success: output.success,
                    error: output.error,
                    relay_statuses,
                    connected_before: output.connected_before,
                    connected_after: output.connected_after,
                    filter_count: output.filter_count,
                },
            )));
        });
    }

    pub(super) fn handle_protocol_subscription_reconcile_completed(
        &mut self,
        generation: u64,
        token: u64,
        reason: String,
        plan: Option<ProtocolSubscriptionPlan>,
        success: bool,
        error: Option<String>,
        relay_statuses: Vec<(String, RelayStatus)>,
        connected_before: u64,
        connected_after: u64,
        filter_count: u64,
    ) {
        if generation != self.protocol_reconnect_token
            || token != self.protocol_subscription_runtime.reconcile_token
        {
            return;
        }
        self.protocol_subscription_runtime.refresh_in_flight = false;
        self.protocol_subscription_runtime.applying_plan = None;
        let refresh_dirty = self.protocol_subscription_runtime.refresh_dirty;
        let force_reconnect_dirty = self.protocol_subscription_runtime.force_reconnect_dirty;
        self.protocol_subscription_runtime.refresh_dirty = false;
        self.protocol_subscription_runtime.force_reconnect_dirty = false;
        if success {
            self.protocol_subscription_runtime.applied_plan = plan;
        }

        self.apply_relay_statuses(relay_statuses);
        self.push_debug_log(
            "protocol.subscription.reconcile",
            format!(
                "reason={reason} generation={} current_generation={} connected_before={connected_before} connected_after={connected_after} filters={filter_count} success={success} dirty={refresh_dirty} error={}",
                generation,
                self.protocol_reconnect_token,
                error.as_deref().unwrap_or("")
            ),
        );
        if connected_after > 0 {
            self.retry_protocol_engine_pending_outbound("subscription_reconciled");
            self.retry_pending_relay_publishes("subscription_reconciled");
        } else {
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_RECONNECT_CHECK_SECS,
            ));
        }
        if refresh_dirty {
            self.request_protocol_subscription_refresh_inner(false, force_reconnect_dirty);
        } else if !success
            && self.protocol_subscription_runtime.desired_plan
                != self.protocol_subscription_runtime.applied_plan
        {
            self.protocol_subscription_runtime.refresh_dirty = true;
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_RECONNECT_CHECK_SECS,
            ));
        }
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn can_poll_pending_device_invites(&self) -> bool {
        self.logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_keys.is_some())
            .unwrap_or(false)
    }

    pub(super) fn known_message_author_hexes(&self) -> HashSet<String> {
        self.protocol_engine
            .as_ref()
            .map(ProtocolEngine::known_message_author_pubkeys)
            .unwrap_or_default()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect()
    }

    pub(super) fn has_seen_event(&self, event_id: &str) -> bool {
        self.seen_event_ids.contains(event_id)
    }

    pub(super) fn remember_event(&mut self, event_id: String) {
        if !self.seen_event_ids.insert(event_id.clone()) {
            return;
        }

        self.seen_event_order.push_back(event_id);
        while self.seen_event_order.len() > MAX_SEEN_EVENT_IDS {
            if let Some(expired) = self.seen_event_order.pop_front() {
                self.seen_event_ids.remove(&expired);
            }
        }
    }
}

fn normalize_protocol_queued_targets(targets: &mut Vec<String>) {
    targets.retain(|target| !target.is_empty());
    targets.sort();
    targets.dedup();
}

struct ProtocolSubscriptionApplyOutput {
    connected_before: u64,
    connected_after: u64,
    filter_count: u64,
    success: bool,
    error: Option<String>,
}

pub(super) fn build_protocol_subscription_filters(plan: &ProtocolSubscriptionPlan) -> Vec<Filter> {
    let roster_authors = pubkeys_from_hexes(&plan.roster_authors);
    let invite_authors = pubkeys_from_hexes(&plan.invite_authors);
    let message_authors = pubkeys_from_hexes(&plan.message_authors);
    let group_sender_key_authors = pubkeys_from_hexes(&plan.group_sender_key_authors);
    let invite_response_recipients = plan
        .invite_response_recipient
        .as_deref()
        .map(pubkeys_from_comma_separated_hexes)
        .unwrap_or_default();

    let mut filters = Vec::new();
    if !roster_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .authors(roster_authors)
                .identifier(NDR_APP_KEYS_D_TAG),
        );
    }
    if !invite_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_EVENT_KIND as u16))
                .authors(invite_authors.clone())
                .custom_tag(SingleLetterTag::lowercase(Alphabet::L), NDR_INVITES_L_TAG),
        );
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                .authors(invite_authors),
        );
    }
    if !message_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
                .authors(message_authors),
        );
    }
    if !group_sender_key_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(GROUP_SENDER_KEY_MESSAGE_KIND as u16))
                .authors(group_sender_key_authors),
        );
    }
    if !invite_response_recipients.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                .pubkeys(invite_response_recipients),
        );
    }
    filters
}

fn pubkeys_from_hexes(hexes: &[String]) -> Vec<PublicKey> {
    hexes
        .iter()
        .filter_map(|hex| PublicKey::parse(hex).ok())
        .collect()
}

fn pubkeys_from_comma_separated_hexes(hexes: &str) -> Vec<PublicKey> {
    hexes
        .split(',')
        .filter(|hex| !hex.is_empty())
        .filter_map(|hex| PublicKey::parse(hex).ok())
        .collect()
}

async fn current_client_relay_statuses(client: &Client) -> Vec<(String, RelayStatus)> {
    client
        .relays()
        .await
        .into_iter()
        .map(|(relay_url, relay)| {
            let relay_url = normalize_nostr_relay_url(&relay_url.to_string())
                .unwrap_or_else(|_| relay_url.to_string());
            (relay_url, relay.status())
        })
        .collect()
}

async fn subscribe_protocol_filters_with_id(
    client: &Client,
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
) -> Result<(), String> {
    let relays = client.relays().await;
    let mut attempted = 0usize;
    let mut accepted = 0usize;
    let mut last_error = None;
    for relay in relays.values() {
        if relay.status() != RelayStatus::Connected {
            continue;
        }
        attempted = attempted.saturating_add(1);
        match relay
            .subscribe_with_id(
                subscription_id.clone(),
                filters.clone(),
                SubscribeOptions::default(),
            )
            .await
        {
            Ok(()) => accepted = accepted.saturating_add(1),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if accepted > 0 {
        Ok(())
    } else if attempted == 0 {
        Err("no connected relays".to_string())
    } else {
        Err(last_error.unwrap_or_else(|| "no relay accepted subscription".to_string()))
    }
}

async fn fetch_events_for_filters(
    client: &Client,
    filters: Vec<Filter>,
    timeout: Duration,
) -> Result<Vec<Event>, String> {
    use tokio::task::JoinSet;

    let mut tasks = JoinSet::new();
    for filter in filters {
        let client = client.clone();
        tasks.spawn(async move { client.fetch_events(filter, timeout).await });
    }

    let mut any_success = false;
    let mut last_error = None;
    let mut seen_event_ids = HashSet::new();
    let mut collected = Vec::new();

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(events)) => {
                any_success = true;
                for event in events.iter() {
                    if seen_event_ids.insert(event.id) {
                        collected.push(event.clone());
                    }
                }
            }
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }
    }

    if any_success {
        Ok(collected)
    } else {
        Err(last_error.unwrap_or_else(|| "no protocol filters fetched".to_string()))
    }
}

async fn wait_for_connected_relays(client: &Client, timeout: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let connected = client
            .relays()
            .await
            .values()
            .filter(|relay| relay.status() == RelayStatus::Connected)
            .count();
        if connected > 0 || tokio::time::Instant::now() >= deadline {
            return connected;
        }
        sleep(Duration::from_millis(100)).await;
    }
}
