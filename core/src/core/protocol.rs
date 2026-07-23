use super::*;
use crate::core::projection::relay_connection_status;

mod backfill;
mod retry_helpers;
mod subscription_helpers;

use self::backfill::{
    direct_message_history_filter, direct_message_recipient_history_filter,
    group_sender_key_history_filter, protocol_event_summary,
    NEW_MESSAGE_AUTHOR_DELAYED_BACKFILL_MS, PROTOCOL_BACKFILL_AUTHOR_BATCH_SIZE,
};
pub(super) use self::subscription_helpers::{
    build_protocol_subscription_filters, fetch_events_for_filters,
};
use self::subscription_helpers::{
    current_client_relay_statuses, pubkeys_from_comma_separated_hexes, pubkeys_from_hexes,
    subscribe_protocol_filters_with_id, wait_for_connected_relays, ProtocolSubscriptionApplyOutput,
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
            if self.drain_queued_direct_text_messages(reason) {
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
            }
            self.schedule_fast_protocol_retry_if_pending();
            return;
        }
        let published = batch
            .group_result
            .effects
            .iter()
            .chain(batch.effects.iter())
            .filter(|effect| matches!(effect, ProtocolEffect::Publish(_)))
            .count();
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
        self.drain_queued_direct_text_messages(reason);
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn retry_protocol_engine_pending_work(&mut self, reason: &'static str) {
        if self.pending_private_invite_cleanup_retry {
            self.retry_all_pending_private_invite_responses();
        }
        if let Some(owner) = self
            .pending_outgoing_invite_acceptance
            .as_ref()
            .map(|pending| pending.claimed_owner)
        {
            self.resume_pending_outgoing_invite_acceptance(owner);
        }
        let Some(protocol_engine) = self.protocol_engine.as_mut() else {
            return;
        };
        let now = NdrUnixSeconds(unix_now().get());
        if !protocol_engine.has_due_pending_retry_work(now) {
            self.process_protocol_engine_retry_batch(reason, ProtocolRetryBatch::default());
            return;
        }
        let results = match protocol_engine.retry_pending_protocol(now) {
            Ok(results) => results,
            Err(error) => {
                self.push_debug_log("appcore.protocol.retry.error", error.to_string());
                return;
            }
        };
        if results.is_empty() {
            self.schedule_fast_protocol_retry_if_pending();
            return;
        }
        self.process_protocol_engine_retry_batch(reason, results);
    }

    pub(super) fn append_protocol_retry_batch(
        target: &mut ProtocolRetryBatch,
        mut source: ProtocolRetryBatch,
    ) {
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
        owners.extend(
            self.pending_private_invite_responses
                .values()
                .map(|pending| pending.claimed_owner.to_hex()),
        );
        if let Some(pending) = self.pending_outgoing_invite_acceptance.as_ref() {
            owners.insert(pending.claimed_owner.to_hex());
        }
        owners.extend(self.app_keys.keys().cloned());
        if let Some(logged_in) = self.logged_in.as_ref() {
            owners.insert(logged_in.owner_pubkey.to_hex());
        }
        owners
    }

    pub(super) fn schedule_tracked_peer_catch_up(&mut self, after: Duration) {
        let due_at = Instant::now() + after;
        if self
            .protocol_subscription_runtime
            .tracked_peer_catch_up_due_at
            .is_some_and(|existing| existing <= due_at)
        {
            return;
        }
        self.protocol_subscription_runtime
            .tracked_peer_catch_up_due_at = Some(due_at);
        self.protocol_subscription_runtime
            .tracked_peer_catch_up_token = self
            .protocol_subscription_runtime
            .tracked_peer_catch_up_token
            .wrapping_add(1);
        let token = self
            .protocol_subscription_runtime
            .tracked_peer_catch_up_token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep_until(due_at).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::FetchTrackedPeerCatchUp { token },
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
        self.state.busy.syncing_network = subscription.protocol_fetch_in_flight
            || subscription.protocol_author_backfill_in_flight > 0
            || subscription.refresh_in_flight
            || subscription.refresh_dirty
            || subscription.applying_plan.is_some()
            || subscription.desired_plan != subscription.applied_plan;
    }

    pub(super) fn fetch_recent_messages_for_author(&mut self, author_pubkey: PublicKey) {
        self.fetch_recent_messages_for_authors(vec![author_pubkey]);
    }

    fn fetch_recent_messages_for_authors(&mut self, author_pubkeys: Vec<PublicKey>) {
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
        let filters = author_pubkeys
            .chunks(PROTOCOL_BACKFILL_AUTHOR_BATCH_SIZE)
            .map(|author_chunk| direct_message_history_filter(author_chunk.to_vec()))
            .collect::<Vec<_>>();
        self.spawn_protocol_author_backfills(client, relay_urls, filters, "direct_message_authors");
    }

    fn fetch_recent_messages_for_recipients(&mut self, recipient_pubkeys: Vec<PublicKey>) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if recipient_pubkeys.is_empty() {
            return;
        }
        let now = unix_now();
        let filters = recipient_pubkeys
            .chunks(PROTOCOL_BACKFILL_AUTHOR_BATCH_SIZE)
            .map(|recipient_chunk| {
                direct_message_recipient_history_filter(recipient_chunk.to_vec(), now)
            })
            .collect::<Vec<_>>();
        self.spawn_protocol_author_backfills(
            client,
            relay_urls,
            filters,
            "direct_message_recipients",
        );
    }

    fn spawn_protocol_author_backfills(
        &mut self,
        client: Client,
        relay_urls: Vec<RelayUrl>,
        filters: Vec<Filter>,
        reason: &'static str,
    ) {
        if filters.is_empty() {
            return;
        }
        self.protocol_subscription_runtime
            .protocol_author_backfill_in_flight = self
            .protocol_subscription_runtime
            .protocol_author_backfill_in_flight
            .saturating_add(filters.len() as u64);
        self.refresh_protocol_sync_busy();
        self.emit_state();
        self.push_debug_log(
            "protocol.author_backfill.fetch",
            format!("reason={reason} filters={}", filters.len()),
        );
        for filter in filters {
            let tx = self.core_sender.clone();
            let client = client.clone();
            let relay_urls = relay_urls.clone();
            self.runtime.spawn(async move {
                ensure_session_relays_configured(&client, &relay_urls).await;
                connect_client_with_timeout(&client, Duration::from_secs(5)).await;
                match client.fetch_events(filter, Duration::from_secs(5)).await {
                    Ok(events) => {
                        let collected = events.iter().cloned().collect::<Vec<_>>();
                        let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                            category: "protocol.author_backfill.result".to_string(),
                            detail: format!(
                                "reason={reason} events={} summary={}",
                                collected.len(),
                                protocol_event_summary(&collected)
                            ),
                        })));
                        if !collected.is_empty() {
                            let _ = tx.send(CoreMsg::Internal(Box::new(
                                InternalEvent::FetchCatchUpEvents(collected),
                            )));
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                            category: "protocol.author_backfill.error".to_string(),
                            detail: format!("reason={reason} error={error}"),
                        })));
                    }
                }
                let _ = tx.send(CoreMsg::Internal(Box::new(
                    InternalEvent::ProtocolAuthorBackfillComplete {
                        reason: reason.to_string(),
                    },
                )));
            });
        }
    }

    fn schedule_new_message_author_backfill(&self, author_pubkeys: Vec<PublicKey>) {
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
                let filter = direct_message_history_filter(authors);
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
        &mut self,
        author_pubkey: PublicKey,
    ) {
        self.fetch_recent_group_sender_key_messages_for_authors(vec![author_pubkey]);
    }

    fn fetch_recent_group_sender_key_messages_for_authors(
        &mut self,
        author_pubkeys: Vec<PublicKey>,
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
        let filters = author_pubkeys
            .chunks(PROTOCOL_BACKFILL_AUTHOR_BATCH_SIZE)
            .map(|author_chunk| group_sender_key_history_filter(author_chunk.to_vec()))
            .collect::<Vec<_>>();
        self.spawn_protocol_author_backfills(
            client,
            relay_urls,
            filters,
            "group_sender_key_authors",
        );
    }

    pub(super) fn fetch_recent_messages_for_tracked_peers(&mut self) {
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
        let direct_recipients = self
            .protocol_subscription_runtime
            .desired_plan
            .as_ref()
            .map(|plan| {
                plan.message_recipients
                    .iter()
                    .filter_map(|hex| PublicKey::parse(hex).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| self.protocol_message_recipient_pubkeys());
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
        if let [author] = direct_authors.as_slice() {
            self.fetch_recent_messages_for_author(*author);
        } else {
            self.fetch_recent_messages_for_authors(direct_authors);
        }
        self.fetch_recent_messages_for_recipients(direct_recipients);
        self.fetch_recent_group_sender_key_messages_for_authors(group_authors);
    }

    pub(super) fn recent_protocol_filters(&self, now: UnixSeconds) -> Vec<Filter> {
        self.recent_protocol_filters_inner(now, true)
    }

    pub(super) fn recent_protocol_metadata_filters(&self, now: UnixSeconds) -> Vec<Filter> {
        self.recent_protocol_filters_inner(now, false)
    }

    fn recent_protocol_filters_inner(
        &self,
        now: UnixSeconds,
        include_message_history: bool,
    ) -> Vec<Filter> {
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
        let invite_authors = pubkeys_from_hexes(&plan.invite_authors);
        filters.extend(build_protocol_discovery_filters(
            owners.clone(),
            invite_authors.clone(),
            DEVICE_INVITE_DISCOVERY_LIMIT,
        ));
        if include_message_history {
            let message_authors = pubkeys_from_hexes(&plan.message_authors);
            if !message_authors.is_empty() {
                filters.push(direct_message_history_filter(message_authors));
            }

            let message_recipients = pubkeys_from_hexes(&plan.message_recipients);
            if !message_recipients.is_empty() {
                filters.push(direct_message_recipient_history_filter(
                    message_recipients,
                    now,
                ));
            }

            let group_sender_key_authors = pubkeys_from_hexes(&plan.group_sender_key_authors);
            if !group_sender_key_authors.is_empty() {
                filters.push(group_sender_key_history_filter(group_sender_key_authors));
            }
        }

        if !plan.group_roster_group_ids.is_empty() {
            let filter = build_group_roster_fact_filter(
                plan.group_roster_group_ids.iter(),
                pubkeys_from_hexes(&plan.group_roster_authors),
            );
            filters.push(filter);
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
            self.app_keys.get(owner_hex).is_some_and(|known| {
                PublicKey::parse(owner_hex).is_ok_and(|owner_pubkey| {
                    engine.active_roster_session_count_for_owner(owner_pubkey) < known.devices.len()
                })
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

    pub(super) fn fetch_recent_protocol_state(&mut self) -> bool {
        self.fetch_recent_protocol_state_inner(true)
    }

    pub(super) fn fetch_recent_protocol_metadata_state(&mut self) -> bool {
        self.fetch_recent_protocol_state_inner(false)
    }

    fn fetch_recent_protocol_state_inner(&mut self, include_message_history: bool) -> bool {
        if self.protocol_subscription_runtime.protocol_fetch_in_flight {
            self.push_debug_log("protocol.catch_up.skip", "fetch already in flight");
            return false;
        }
        if let Some(delay) = self.protocol_fetch_rate_limit_delay() {
            self.push_debug_log(
                "protocol.catch_up.skip",
                format!("rate limited for {}ms", delay.as_millis()),
            );
            self.schedule_tracked_peer_catch_up(delay);
            return false;
        }
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .filter(|logged_in| !logged_in.relay_urls.is_empty())
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return false;
        };
        let now = unix_now();
        let filters = if include_message_history {
            self.recent_protocol_filters(now)
        } else {
            self.recent_protocol_metadata_filters(now)
        };
        if filters.is_empty() {
            return false;
        }
        self.push_debug_log(
            "protocol.catch_up.fetch",
            format!(
                "filters={} messages={include_message_history}",
                filters.len()
            ),
        );
        self.state.busy.syncing_network = true;
        self.protocol_subscription_runtime.protocol_fetch_in_flight = true;
        self.protocol_subscription_runtime
            .protocol_fetch_last_started_at = Some(Instant::now());

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

    pub(super) fn fetch_pending_device_invites_for_local_owner(&mut self) -> bool {
        self.fetch_recent_protocol_metadata_state()
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
            self.retry_protocol_engine_pending_work("relay_transport_connected");
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
        let added_message_author_pubkeys = added_message_authors
            .into_iter()
            .filter_map(|author_hex| PublicKey::parse(&author_hex).ok())
            .collect::<Vec<_>>();
        if !added_message_author_pubkeys.is_empty() {
            self.fetch_recent_messages_for_authors(added_message_author_pubkeys.clone());
            self.schedule_new_message_author_backfill(added_message_author_pubkeys);
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
                self.fetch_recent_group_sender_key_messages_for_author(author);
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
            .map(relay_connection_status)
            .unwrap_or("offline");
        let next_visible_status = relay_connection_status(status);
        let was_connected = self.relay_connected_count > 0;
        self.relay_status_by_url
            .insert(normalized_relay_url.clone(), status);
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
                self.request_user_discovery_refresh(false);
                self.reconcile_protocol_subscriptions("relay_connected", false);
                self.fetch_recent_protocol_state();
                self.fetch_recent_messages_for_tracked_peers();
                self.retry_protocol_engine_pending_work("relay_connected");
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
        let pending_protocol_retry_needed = self.has_pending_protocol_engine_retry_work();
        if !has_subscription_work && !has_pending_relay_publishes && !pending_protocol_retry_needed
        {
            return;
        }
        self.refresh_relay_connection_status_from_cached_statuses();
        let connected_relays = self.relay_connected_count as usize;
        self.push_debug_log(
            "protocol.liveness",
            format!(
                "connected={connected_relays} pending_protocol={pending_protocol_retry_needed} pending_publishes={}",
                self.pending_relay_publishes.len()
            ),
        );
        if has_subscription_work {
            self.reconcile_protocol_subscriptions("liveness_check", true);
        }
        if pending_protocol_retry_needed {
            self.retry_protocol_engine_pending_work("liveness_check");
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

    // This mirrors the fields of InternalEvent::ProtocolSubscriptionReconcileCompleted;
    // keeping the boundary flat makes event dispatch and protocol diagnostics explicit.
    #[allow(clippy::too_many_arguments)]
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
            self.retry_protocol_engine_pending_work("subscription_reconciled");
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
