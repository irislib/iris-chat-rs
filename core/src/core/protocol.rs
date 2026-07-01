use super::*;
use crate::core::projection::relay_connection_status;

mod retry_helpers;
mod subscription_helpers;

pub(super) use self::subscription_helpers::build_protocol_subscription_filters;
use self::subscription_helpers::{
    current_client_relay_statuses, subscribe_protocol_filters_with_id, wait_for_connected_relays,
    ProtocolSubscriptionApplyOutput,
};

const PROTOCOL_SUBSCRIPTION_ID: &str = "ndr-protocol";
const PROTOCOL_SUBSCRIPTION_APPLY_TIMEOUT_SECS: u64 = 8;
const PROTOCOL_SUBSCRIPTION_LIVENESS_CHECK_SECS: u64 = 30;
pub(super) const PROTOCOL_RECONNECT_CHECK_SECS: u64 = 2;
const RELAY_TRANSPORT_RETRY_BACKOFF_SECS: [u64; 5] = [2, 5, 15, 30, 60];
const RELAY_CONNECT_STALE_AFTER: Duration = Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS * 4);

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
        let result =
            protocol_engine.send_direct_unsigned_event(peer, chat_id, unsigned, unix_now());
        self.handle_protocol_direct_send_result(chat_id, reason, result)
    }

    pub(super) fn send_protocol_engine_unsigned_event_to_peer_only(
        &mut self,
        peer: PublicKey,
        chat_id: &str,
        unsigned: UnsignedEvent,
        reason: &'static str,
    ) -> bool {
        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        let result = protocol_engine.send_direct_unsigned_event_to_peer_only(
            peer,
            chat_id,
            unsigned,
            unix_now(),
        );
        self.handle_protocol_direct_send_result(chat_id, reason, result)
    }

    pub(super) fn send_protocol_engine_unsigned_event_to_local_siblings(
        &mut self,
        conversation_owner: PublicKey,
        chat_id: &str,
        unsigned: UnsignedEvent,
        reason: &'static str,
    ) -> bool {
        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        let result = protocol_engine.send_local_sibling_unsigned_event(
            conversation_owner,
            chat_id,
            unsigned,
            unix_now(),
        );
        self.handle_protocol_direct_send_result(chat_id, reason, result)
    }

    fn handle_protocol_direct_send_result(
        &mut self,
        chat_id: &str,
        reason: &'static str,
        result: anyhow::Result<ProtocolDirectSendResult>,
    ) -> bool {
        match result {
            Ok(result) => {
                self.push_debug_log(
                    "appcore.protocol.send",
                    format!(
                        "reason={reason} chat_id={chat_id} event_ids={}",
                        result.event_ids.len()
                    ),
                );
                self.process_protocol_engine_effects(result.effects);
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
        if batch.is_empty() {
            return;
        }
        let mut published = 0usize;
        let mut direct_effects = Vec::new();
        let mut direct_message_refs = Vec::new();
        for result in batch.direct_results {
            published = published.saturating_add(result.event_ids.len());
            direct_effects.extend(result.effects);
            direct_message_refs.push((result.chat_id, result.message_id));
        }
        self.process_protocol_engine_effects(direct_effects);
        for (chat_id, message_id) in direct_message_refs {
            self.sync_message_delivery_trace(&chat_id, &message_id);
            self.reconcile_outgoing_message_delivery(&chat_id, &message_id);
        }
        for group_event in batch.group_result.events {
            self.apply_group_decrypted_event(group_event);
        }
        self.process_protocol_engine_effects(batch.group_result.effects);
        self.process_protocol_engine_effects(batch.effects);
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
            format!("reason={reason} published={published}"),
        );
        self.request_protocol_subscription_refresh();
        self.schedule_fast_protocol_retry_if_pending();
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
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
        if let Some(protocol_engine) = self.protocol_engine.as_ref() {
            owners.extend(
                protocol_engine
                    .queued_owner_claim_targets()
                    .into_iter()
                    .map(|target| {
                        target
                            .strip_prefix("owner:")
                            .unwrap_or(target.as_str())
                            .to_string()
                    }),
            );
        }
        owners.extend(self.app_keys.keys().cloned());
        if let Some(logged_in) = self.logged_in.as_ref() {
            owners.insert(logged_in.owner_pubkey.to_hex());
        }
        owners
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
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        let token = self.protocol_liveness_token;
        let tx = self.priority_sender.clone();
        self.runtime.spawn(async move {
            sleep_until(due_at).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::ProtocolSubscriptionLivenessCheck { token },
            )));
        });
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

    pub(super) fn refresh_protocol_sync_busy(&mut self) {
        let subscription = &self.protocol_subscription_runtime;
        self.state.busy.syncing_network = subscription.refresh_in_flight
            || subscription.refresh_dirty
            || subscription.applying_plan.is_some()
            || subscription.desired_plan != subscription.applied_plan;
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
        if let Some(logged_in) = self.logged_in.as_ref() {
            pubkeys.push(logged_in.device_keys.public_key());
        }
        if let Some(local_invite_pubkey) = self
            .protocol_engine
            .as_ref()
            .and_then(ProtocolEngine::local_invite_response_pubkey)
        {
            pubkeys.push(local_invite_pubkey);
        }
        pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
        pubkeys.dedup();
        pubkeys
    }

    pub(super) fn message_recipient_bootstrap_needed(&self) -> bool {
        let Some(engine) = self.protocol_engine.as_ref() else {
            return false;
        };
        self.tracked_peer_owner_hexes().iter().any(|owner_hex| {
            self.app_keys.contains_key(owner_hex)
                && PublicKey::parse(owner_hex).is_ok_and(|owner_pubkey| {
                    engine
                        .message_author_pubkeys_for_owner(owner_pubkey)
                        .is_empty()
                })
        })
    }

    pub(super) fn protocol_message_recipient_pubkeys(&self) -> Vec<PublicKey> {
        if !self.message_recipient_bootstrap_needed() {
            return Vec::new();
        }
        self.logged_in
            .as_ref()
            .map(|logged_in| vec![logged_in.device_keys.public_key()])
            .unwrap_or_default()
    }

    pub(super) fn fetch_pending_device_invites_for_local_owner(&mut self) {
        self.request_protocol_subscription_refresh();
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

        let mut watch_urls = HashSet::new();
        for relay_url in &relay_urls {
            let normalized = normalize_nostr_relay_url(&relay_url.to_string())
                .unwrap_or_else(|_| relay_url.to_string());
            if self.relay_status_watch_urls.insert(normalized.clone()) {
                watch_urls.insert(normalized);
            }
        }
        if watch_urls.is_empty() {
            return;
        }

        let generation = self.relay_status_watch_generation;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            for (relay_url, relay) in client.relays().await {
                let relay_url = normalize_nostr_relay_url(&relay_url.to_string())
                    .unwrap_or_else(|_| relay_url.to_string());
                if !watch_urls.contains(&relay_url) {
                    continue;
                }
                let _ = tx.send(CoreMsg::Internal(Box::new(
                    InternalEvent::RelayStatusChanged {
                        relay_url: relay_url.clone(),
                        status: relay.status(),
                        generation,
                    },
                )));
                let mut notifications = relay.notifications();
                let tx = tx.clone();
                tokio::spawn(async move {
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
        });
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
        if self.relay_transport_runtime.connect_in_flight
            && self
                .relay_transport_runtime
                .connect_started_at
                .is_some_and(|started_at| started_at.elapsed() >= RELAY_CONNECT_STALE_AFTER)
        {
            self.relay_transport_runtime.connect_in_flight = false;
            self.relay_transport_runtime.connect_dirty = false;
            self.relay_transport_runtime.force_reconnect_dirty = false;
            self.relay_transport_runtime.connect_started_at = None;
            self.push_debug_log(
                "relay.transport.connect",
                format!("reason={reason} reset_stale_in_flight=true"),
            );
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
        self.relay_transport_runtime.connect_started_at = Some(Instant::now());
        self.relay_transport_runtime.connect_token =
            self.relay_transport_runtime.connect_token.wrapping_add(1);
        self.relay_transport_runtime.last_connect_reason = Some(reason.clone());
        let token = self.relay_transport_runtime.connect_token;
        let tx = self.priority_sender.clone();
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
        self.relay_transport_runtime.connect_started_at = None;
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
        let delay = Duration::from_secs(
            RELAY_TRANSPORT_RETRY_BACKOFF_SECS
                .get(attempt_index)
                .copied()
                .unwrap_or(1),
        );
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
            self.refresh_protocol_sync_busy();
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
            self.refresh_protocol_sync_busy();
            return;
        }

        let previous_desired = self.protocol_subscription_runtime.desired_plan.clone();
        let desired_plan = self.compute_protocol_subscription_plan();
        let plan_changed = previous_desired != desired_plan;
        self.note_protocol_plan_author_changes(previous_desired.as_ref(), desired_plan.as_ref());
        self.protocol_subscription_runtime.desired_plan = desired_plan.clone();
        self.refresh_protocol_sync_busy();

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
        let message_authors = sorted_hexes(self.subscribable_message_author_hexes());
        let message_recipients = self
            .protocol_message_recipient_pubkeys()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<HashSet<_>>();
        let message_recipients = sorted_hexes(message_recipients);
        let group_sender_key_authors = self
            .protocol_engine
            .as_ref()
            .map(ProtocolEngine::known_group_sender_event_pubkeys)
            .unwrap_or_default()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<HashSet<_>>();
        let group_sender_key_authors = sorted_hexes(group_sender_key_authors);
        let group_roster_group_ids = self.groups.keys().cloned().collect::<HashSet<_>>();
        let group_roster_group_ids = sorted_hexes(group_roster_group_ids);
        let group_roster_authors = self
            .groups
            .values()
            .flat_map(|group| group.admins.iter())
            .filter_map(|owner| PublicKey::from_slice(&owner.to_bytes()).ok())
            .map(|pubkey| pubkey.to_hex())
            .collect::<HashSet<_>>();
        let group_roster_authors = sorted_hexes(group_roster_authors);
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
            || !message_recipients.is_empty()
            || !group_roster_group_ids.is_empty()
            || !group_sender_key_authors.is_empty()
            || invite_response_recipient.is_some();
        has_filters.then_some(ProtocolSubscriptionPlan {
            runtime_subscriptions: vec![PROTOCOL_SUBSCRIPTION_ID.to_string()],
            roster_authors,
            invite_authors,
            message_authors,
            message_recipients,
            group_roster_group_ids,
            group_roster_authors,
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
        if !added_group_authors.is_empty() {
            self.mark_mobile_push_dirty();
        }
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

    #[cfg(test)]
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

    #[cfg(test)]
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
        if token != self.protocol_liveness_token {
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
        self.refresh_relay_connection_status_from_cached_statuses();
        let connected_relays = self.relay_connected_count as usize;
        self.push_debug_log(
            "protocol.liveness",
            format!(
                "connected={connected_relays} pending_publishes={}",
                self.pending_relay_publishes.len()
            ),
        );
        if has_subscription_work {
            self.reconcile_protocol_subscriptions("liveness_check", true);
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
            self.refresh_protocol_sync_busy();
            self.push_debug_log(
                "protocol.subscription.reconcile",
                format!("reason={reason} deferred=in_flight"),
            );
            return;
        }
        self.refresh_relay_connection_status_from_cached_statuses();
        if self.relay_connected_count == 0 {
            self.protocol_subscription_runtime.refresh_dirty = true;
            self.protocol_subscription_runtime.force_reconnect_dirty |= force_reconnect_if_offline;
            self.refresh_protocol_sync_busy();
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
        self.refresh_protocol_sync_busy();
        self.protocol_subscription_runtime.reconcile_token = self
            .protocol_subscription_runtime
            .reconcile_token
            .wrapping_add(1);
        let token = self.protocol_subscription_runtime.reconcile_token;
        let generation = self.protocol_reconnect_token;
        let tx = self.priority_sender.clone();
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
        if token != self.protocol_subscription_runtime.reconcile_token {
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
        self.refresh_protocol_sync_busy();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn can_poll_pending_device_invites(&self) -> bool {
        self.logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_keys.is_some())
            .unwrap_or(false)
    }

    /// Walks every active and inactive session and returns the
    /// expected event-author pubkeys (hex), narrowed to peers the
    /// user actually wants traffic from: blocked owners are always
    /// excluded, and when the unknown-users toggle is off, owners
    /// that aren't in the accepted set are excluded too. The owner
    /// gate is applied per-session (so we never include any of a
    /// blocked owner's device-ephemeral event authors). The result
    /// feeds both the nostr relay subscription's `authors` filter and
    /// the mobile-push subscription body.
    pub(super) fn subscribable_message_author_hexes(&self) -> HashSet<String> {
        let accept_unknown = self.preferences.accept_unknown_direct_messages;
        let blocked: HashSet<String> = self
            .preferences
            .blocked_owner_pubkeys
            .iter()
            .cloned()
            .collect();
        let accepted: HashSet<String> = self
            .preferences
            .accepted_owner_pubkeys
            .iter()
            .cloned()
            .collect();
        let Some(engine) = self.protocol_engine.as_ref() else {
            return HashSet::new();
        };
        engine
            .message_author_pubkeys_filtered(|owner| {
                let hex = owner.to_hex();
                if blocked.contains(&hex) {
                    return false;
                }
                accept_unknown || accepted.contains(&hex)
            })
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
