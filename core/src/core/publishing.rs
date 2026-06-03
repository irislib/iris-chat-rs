use super::*;
use crate::core::protocol::PROTOCOL_RECONNECT_CHECK_SECS;

const PENDING_RELAY_DRAIN_CONCURRENCY: usize = 4;
const PENDING_RELAY_DRAIN_BATCH_SIZE: usize = 16;
const PENDING_RELAY_DRAIN_STALE_AFTER: Duration = RELAY_PUBLISH_ATTEMPT_TIMEOUT;
const PENDING_RELAY_PUBLISH_IN_PROGRESS: &str = "publish attempt in progress";
const LOCAL_INVITE_PUBLISH_LABEL: &str = "invite";

fn send_nearby_published_event(update_tx: &Sender<AppUpdate>, event: &Event) {
    let Ok(event_json) = serde_json::to_string(event) else {
        return;
    };
    let _ = update_tx.send(AppUpdate::NearbyPublishedEvent {
        event_id: event.id.to_string(),
        kind: event.kind.as_u16() as u32,
        created_at_secs: event.created_at.as_secs(),
        event_json,
    });
}

impl AppCore {
    pub(super) fn emit_nearby_published_event(&self, event: &Event) {
        send_nearby_published_event(&self.update_tx, event);
    }

    pub(super) fn publish_runtime_event(
        &mut self,
        event: Event,
        label: &'static str,
        completion: Option<(String, String)>,
    ) -> bool {
        let (inner_event_id, chat_id) = completion
            .map(|(inner_event_id, chat_id)| (Some(inner_event_id), Some(chat_id)))
            .unwrap_or((None, None));
        self.publish_runtime_event_with_metadata(event, label, chat_id, inner_event_id)
    }

    pub(super) fn publish_protocol_event(&mut self, publish: ProtocolPublish) -> bool {
        self.publish_runtime_event_with_metadata(
            publish.event,
            APPCORE_PROTOCOL_LABEL,
            Some(publish.chat_id),
            publish.inner_event_id,
        )
    }

    fn publish_runtime_event_with_metadata(
        &mut self,
        event: Event,
        label: &'static str,
        chat_id: Option<String>,
        inner_event_id: Option<String>,
    ) -> bool {
        if self.defer_owner_app_keys_publish && is_app_keys_event(&event) {
            self.push_debug_log(
                "publish.runtime",
                "label=runtime skipped=defer_owner_app_keys".to_string(),
            );
            return false;
        }
        self.remember_event(event.id.to_string());
        self.emit_nearby_published_event(&event);
        let event_id = event.id.to_string();
        let stored = self.remember_pending_relay_publish(&event, label, chat_id, inner_event_id);
        if !stored {
            return false;
        }
        let Some(relay_urls) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.relay_urls.clone())
        else {
            return false;
        };
        if relay_urls.is_empty() {
            self.handle_relay_publish_finished(
                event_id,
                false,
                Vec::new(),
                format!("label={label} success=false relays=0 skipped=no_servers"),
            );
            return true;
        }

        self.retry_pending_relay_publishes(label);
        true
    }

    fn remember_pending_relay_publish(
        &mut self,
        event: &Event,
        label: &str,
        chat_id: Option<String>,
        inner_event_id: Option<String>,
    ) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        let owner_pubkey_hex = logged_in.owner_pubkey.to_hex();
        let event_json = match serde_json::to_string(event) {
            Ok(json) => json,
            Err(error) => {
                self.push_debug_log("publish.runtime.queue", format!("serialize_failed={error}"));
                return false;
            }
        };
        let pending = PendingRelayPublish {
            owner_pubkey_hex,
            event_id: event.id.to_string(),
            label: label.to_string(),
            event_json,
            inner_event_id,
            chat_id,
            created_at_secs: event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        };
        if !self.prune_or_skip_superseded_app_keys_publish(event) {
            return false;
        }
        if !self.prune_or_skip_superseded_local_invite_publish(&pending, event) {
            return false;
        }
        if let Err(error) = self.app_store.upsert_pending_relay_publish(&pending) {
            self.push_debug_log("publish.runtime.queue", format!("store_failed={error}"));
            return false;
        }
        if let (Some(message_id), Some(chat_id)) = (
            pending.inner_event_id.as_deref(),
            pending.chat_id.as_deref(),
        ) {
            self.record_message_outer_event(chat_id, message_id, &pending.event_id);
        }
        self.pending_relay_publishes
            .insert(pending.event_id.clone(), pending);
        if let Some(pending) = self.pending_relay_publishes.get(&event.id.to_string()) {
            if let (Some(message_id), Some(chat_id)) =
                (pending.inner_event_id.clone(), pending.chat_id.clone())
            {
                self.sync_message_delivery_trace(&chat_id, &message_id);
            }
        }
        true
    }

    pub(super) fn retry_pending_relay_publishes(&mut self, reason: &str) {
        if self.pending_relay_publishes.is_empty() {
            return;
        }
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if relay_urls.is_empty() {
            self.push_debug_log(
                "publish.runtime.retry",
                format!(
                    "reason={reason} skipped=no_servers pending={}",
                    self.pending_relay_publishes.len()
                ),
            );
            return;
        }
        if self.relay_transport_runtime.publish_drain_in_flight
            && self
                .relay_transport_runtime
                .publish_drain_started_at
                .is_some_and(|started_at| started_at.elapsed() >= PENDING_RELAY_DRAIN_STALE_AFTER)
        {
            let inflight = self.pending_relay_publish_inflight.len();
            self.pending_relay_publish_inflight.clear();
            self.relay_transport_runtime.publish_drain_in_flight = false;
            self.relay_transport_runtime.publish_drain_dirty = false;
            self.relay_transport_runtime.publish_drain_started_at = None;
            self.push_debug_log(
                "relay.transport.drain",
                format!("reason={reason} reset_stale_in_flight={inflight}"),
            );
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_RECONNECT_CHECK_SECS,
            ));
        }
        if self.relay_transport_runtime.publish_drain_in_flight {
            self.relay_transport_runtime.publish_drain_dirty = true;
            self.relay_transport_runtime.last_drain_reason = Some(reason.to_string());
            self.push_debug_log(
                "relay.transport.drain",
                format!(
                    "reason={reason} deferred=in_flight pending={}",
                    self.pending_relay_publishes.len()
                ),
            );
            return;
        }

        self.refresh_relay_connection_status();
        if self.relay_connected_count == 0 {
            self.relay_transport_runtime.publish_drain_dirty = true;
            self.relay_transport_runtime.last_drain_reason = Some(reason.to_string());
            self.push_debug_log(
                "relay.transport.drain",
                format!(
                    "reason={reason} cached_relay_offline=attempt_raw pending={}",
                    self.pending_relay_publishes.len()
                ),
            );
            self.request_relay_connection(format!("publish_drain:{reason}"), false);
            self.schedule_relay_transport_retry(format!("publish_drain_offline:{reason}"));
        }

        let pending = self
            .pending_relay_publishes
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut candidates = Vec::new();
        for pending in pending {
            if self
                .pending_relay_publish_inflight
                .contains(&pending.event_id)
            {
                continue;
            }
            let event = match serde_json::from_str::<Event>(&pending.event_json) {
                Ok(event) => event,
                Err(error) => {
                    self.push_debug_log(
                        "publish.runtime.retry",
                        format!(
                            "event_id={} skipped=invalid_json error={error}",
                            pending.event_id
                        ),
                    );
                    self.forget_pending_relay_publish(&pending.event_id);
                    continue;
                }
            };
            candidates.push((pending, event));
        }
        candidates.sort_by(|(left, _), (right, _)| {
            let left_is_message = left.inner_event_id.is_some() && left.chat_id.is_some();
            let right_is_message = right.inner_event_id.is_some() && right.chat_id.is_some();
            (!left_is_message)
                .cmp(&(!right_is_message))
                .then_with(|| left.created_at_secs.cmp(&right.created_at_secs))
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        let truncated_to_batch = candidates.len() > PENDING_RELAY_DRAIN_BATCH_SIZE;
        if truncated_to_batch {
            candidates.truncate(PENDING_RELAY_DRAIN_BATCH_SIZE);
        }
        if candidates.is_empty() {
            return;
        }

        for (pending, _) in &candidates {
            self.mark_pending_relay_publish_attempt_started(&pending.event_id);
            self.pending_relay_publish_inflight
                .insert(pending.event_id.clone());
        }
        self.relay_transport_runtime.publish_drain_in_flight = true;
        self.relay_transport_runtime.publish_drain_dirty = truncated_to_batch;
        self.relay_transport_runtime.publish_drain_started_at = Some(Instant::now());
        self.relay_transport_runtime.publish_drain_token = self
            .relay_transport_runtime
            .publish_drain_token
            .wrapping_add(1);
        self.relay_transport_runtime.last_drain_reason = Some(reason.to_string());
        let token = self.relay_transport_runtime.publish_drain_token;
        let tx = self.priority_sender.clone();
        let watchdog_tx = self.priority_sender.clone();
        let relay_count = relay_urls.len();
        self.push_debug_log(
            "relay.transport.drain",
            format!(
                "reason={reason} started={} pending={} concurrency={PENDING_RELAY_DRAIN_CONCURRENCY}",
                candidates.len(),
                self.pending_relay_publishes.len()
            ),
        );
        self.schedule_protocol_subscription_liveness_check(PENDING_RELAY_DRAIN_STALE_AFTER);
        self.runtime.spawn(async move {
            tokio::time::sleep(PENDING_RELAY_DRAIN_STALE_AFTER).await;
            let _ = watchdog_tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::RetryPendingRelayPublishes {
                    reason: "publish_drain_watchdog".to_string(),
                },
            )));
        });
        self.runtime.spawn(async move {
            let mut queued = candidates.into_iter();
            let mut join_set = tokio::task::JoinSet::new();
            loop {
                while join_set.len() < PENDING_RELAY_DRAIN_CONCURRENCY {
                    let Some((pending, event)) = queued.next() else {
                        break;
                    };
                    let relay_urls = relay_urls.clone();
                    let client = client.clone();
                    join_set.spawn(async move {
                        let event_id = pending.event_id.clone();
                        let label = pending.label.clone();
                        let result = tokio::time::timeout(
                            RELAY_PUBLISH_ATTEMPT_TIMEOUT,
                            publish_event_to_any_connected_relay(
                                &client,
                                &relay_urls,
                                &event,
                                &label,
                            ),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            Err(anyhow::anyhow!(
                                "{label}: publish attempt timed out after {:?}",
                                RELAY_PUBLISH_ATTEMPT_TIMEOUT
                            ))
                        });
                        let success = result
                            .as_ref()
                            .map(|relays| !relays.is_empty())
                            .unwrap_or(false);
                        let accepted_relays = result
                            .as_ref()
                            .map(|relays| relays.clone())
                            .unwrap_or_default();
                        let detail = match &result {
                            Ok(relays) => {
                                format!(
                                    "label={label} success=true relays={relay_count} accepted_relays={}",
                                    relays.join(",")
                                )
                            }
                            Err(error) => {
                                format!(
                                    "label={label} success=false relays={relay_count} error={error}"
                                )
                            }
                        };
                        RelayPublishDrainResult {
                            event_id,
                            success,
                            relay_urls: accepted_relays,
                            detail,
                        }
                    });
                }
                if join_set.is_empty() {
                    break;
                }
                if let Some(joined) = join_set.join_next().await {
                    if let Ok(result) = joined {
                        let _ = tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::RelayPublishDrainProgress { token, result },
                        )));
                    }
                }
            }
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::RelayPublishDrainFinished {
                    token,
                    results: Vec::new(),
                },
            )));
        });
    }

    fn mark_pending_relay_publish_attempt_started(&mut self, event_id: &str) {
        let Some(pending) = self.pending_relay_publishes.get_mut(event_id) else {
            return;
        };
        pending.attempt_count = pending.attempt_count.saturating_add(1);
        pending.last_error = Some(PENDING_RELAY_PUBLISH_IN_PROGRESS.to_string());
        if let Err(error) = self.app_store.upsert_pending_relay_publish(pending) {
            self.push_debug_log(
                "publish.runtime.queue",
                format!("attempt_update_failed={error}"),
            );
        }
    }

    pub(super) fn handle_relay_publish_drain_progress(
        &mut self,
        token: u64,
        result: RelayPublishDrainResult,
    ) {
        if token != self.relay_transport_runtime.publish_drain_token {
            return;
        }
        if result.success {
            self.relay_transport_runtime.retry_backoff_attempt = 0;
            self.relay_transport_runtime.next_retry_due_at = None;
            self.relay_transport_runtime.next_retry_reason = None;
        }
        let should_retry = self.handle_relay_publish_finished(
            result.event_id,
            result.success,
            result.relay_urls,
            result.detail,
        );
        if should_retry {
            self.schedule_relay_transport_retry("publish_failed");
        }
    }

    pub(super) fn handle_relay_publish_drain_finished(
        &mut self,
        token: u64,
        results: Vec<RelayPublishDrainResult>,
    ) {
        if token != self.relay_transport_runtime.publish_drain_token {
            return;
        }
        self.relay_transport_runtime.publish_drain_in_flight = false;
        self.relay_transport_runtime.publish_drain_started_at = None;
        let drain_dirty = self.relay_transport_runtime.publish_drain_dirty;
        self.relay_transport_runtime.publish_drain_dirty = false;
        let success_count = results.iter().filter(|result| result.success).count();
        self.push_debug_log(
            "relay.transport.drain",
            format!(
                "token={token} completed={} success={} dirty={drain_dirty} pending={}",
                results.len(),
                success_count,
                self.pending_relay_publishes.len()
            ),
        );
        if success_count > 0 {
            self.relay_transport_runtime.retry_backoff_attempt = 0;
            self.relay_transport_runtime.next_retry_due_at = None;
            self.relay_transport_runtime.next_retry_reason = None;
        }
        let mut failed_pending_count = 0usize;
        self.enter_batch();
        for result in results {
            if self.handle_relay_publish_finished(
                result.event_id,
                result.success,
                result.relay_urls,
                result.detail,
            ) {
                failed_pending_count += 1;
            }
        }
        let stranded_inflight = self
            .pending_relay_publish_inflight
            .iter()
            .filter(|event_id| self.pending_relay_publishes.contains_key(*event_id))
            .cloned()
            .collect::<Vec<_>>();
        if !stranded_inflight.is_empty() {
            self.push_debug_log(
                "relay.transport.drain",
                format!(
                    "token={token} recovered_missing_results={}",
                    stranded_inflight.len()
                ),
            );
        }
        for event_id in stranded_inflight {
            if let Some(pending) = self.pending_relay_publishes.get(&event_id).cloned() {
                if self.handle_relay_publish_finished(
                    event_id,
                    false,
                    Vec::new(),
                    format!(
                        "label={} success=false error=drain worker finished without result",
                        pending.label
                    ),
                ) {
                    failed_pending_count += 1;
                }
            }
        }
        let orphan_inflight_count = self.pending_relay_publish_inflight.len();
        if orphan_inflight_count > 0 {
            self.pending_relay_publish_inflight.clear();
            self.push_debug_log(
                "relay.transport.drain",
                format!("token={token} cleared_orphan_inflight={orphan_inflight_count}"),
            );
        }
        if failed_pending_count > 0 {
            self.schedule_relay_transport_retry("publish_failed");
        }
        self.exit_batch();
        if drain_dirty && !self.pending_relay_publishes.is_empty() {
            if failed_pending_count == 0 {
                self.retry_pending_relay_publishes("coalesced_drain");
            } else {
                self.push_debug_log(
                    "relay.transport.drain",
                    format!(
                        "reason=coalesced_drain deferred=backoff failed={failed_pending_count} pending={}",
                        self.pending_relay_publishes.len()
                    ),
                );
            }
        }
    }

    pub(super) fn handle_relay_publish_finished(
        &mut self,
        event_id: String,
        success: bool,
        relay_urls: Vec<String>,
        detail: String,
    ) -> bool {
        self.pending_relay_publish_inflight.remove(&event_id);
        self.push_debug_log("publish.runtime", detail.clone());
        let pending = self.pending_relay_publishes.get(&event_id).cloned();
        let message_ref = pending
            .as_ref()
            .and_then(|pending| Some((pending.chat_id.clone()?, pending.inner_event_id.clone()?)));
        let mut should_retry = false;
        if success {
            self.forget_pending_relay_publish(&event_id);
        } else if let Some(pending) = self.pending_relay_publishes.get_mut(&event_id) {
            if pending.last_error.as_deref() != Some(PENDING_RELAY_PUBLISH_IN_PROGRESS) {
                pending.attempt_count = pending.attempt_count.saturating_add(1);
            }
            pending.last_error = Some(detail.clone());
            if let Err(error) = self.app_store.upsert_pending_relay_publish(pending) {
                self.push_debug_log("publish.runtime.queue", format!("update_failed={error}"));
            }
            should_retry = true;
        }
        if let Some((chat_id, message_id)) = message_ref {
            for relay_url in &relay_urls {
                self.add_message_transport_channel(
                    &chat_id,
                    &message_id,
                    &format!("message server: {relay_url}"),
                );
            }
            if !success {
                self.update_message_delivery(&chat_id, &message_id, DeliveryState::Queued);
            }
            self.sync_message_delivery_trace(&chat_id, &message_id);
            self.reconcile_outgoing_message_delivery(&chat_id, &message_id);
        }
        if success {
            self.reconcile_ready_outgoing_message_deliveries();
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
        should_retry
    }

    fn forget_pending_relay_publish(&mut self, event_id: &str) {
        self.pending_relay_publishes.remove(event_id);
        self.pending_relay_publish_inflight.remove(event_id);
        if let Err(error) = self.app_store.delete_pending_relay_publish(event_id) {
            self.push_debug_log("publish.runtime.queue", format!("delete_failed={error}"));
        }
    }

    fn prune_or_skip_superseded_local_invite_publish(
        &mut self,
        current: &PendingRelayPublish,
        event: &Event,
    ) -> bool {
        if current.label != LOCAL_INVITE_PUBLISH_LABEL
            || event.kind.as_u16() as u32 != INVITE_EVENT_KIND
        {
            return true;
        }

        let current_event_id = event.id.to_string();
        let current_created_at = event.created_at.as_secs();
        let superseded_ids = self
            .pending_relay_publishes
            .values()
            .filter_map(|pending| {
                if pending.event_id == current_event_id
                    || pending.label != LOCAL_INVITE_PUBLISH_LABEL
                    || pending.owner_pubkey_hex != current.owner_pubkey_hex
                {
                    return None;
                }
                let pending_event = serde_json::from_str::<Event>(&pending.event_json).ok()?;
                if pending_event.kind.as_u16() as u32 != INVITE_EVENT_KIND
                    || pending_event.pubkey != event.pubkey
                {
                    return None;
                }
                if pending.created_at_secs > current_created_at {
                    return Some((pending.event_id.clone(), true));
                }
                if pending.created_at_secs < current_created_at {
                    return Some((pending.event_id.clone(), false));
                }
                Some((
                    pending.event_id.clone(),
                    pending.event_id > current_event_id,
                ))
            })
            .collect::<Vec<_>>();
        for (event_id, newer) in superseded_ids {
            if newer {
                self.push_debug_log(
                    "publish.runtime.queue",
                    format!(
                        "label={LOCAL_INVITE_PUBLISH_LABEL} skipped=superseded_by_newer pending_event_id={event_id}"
                    ),
                );
                return false;
            }
            self.forget_pending_relay_publish(&event_id);
        }
        true
    }

    fn prune_or_skip_superseded_app_keys_publish(&mut self, event: &Event) -> bool {
        if !is_app_keys_event(event) {
            return true;
        }

        let Some(d_tag) = event.tags.iter().find_map(|tag| {
            if tag.kind() == nostr_sdk::TagKind::d() {
                tag.content().map(|value| value.to_string())
            } else {
                None
            }
        }) else {
            return true;
        };
        let created_at_secs = event.created_at.as_secs();
        let superseded_ids = self
            .pending_relay_publishes
            .values()
            .filter_map(|pending| {
                if pending.label != "app-keys" {
                    return None;
                }
                let pending_event = serde_json::from_str::<Event>(&pending.event_json).ok()?;
                if !is_app_keys_event(&pending_event) {
                    return None;
                }
                let pending_d_tag = pending_event.tags.iter().find_map(|tag| {
                    if tag.kind() == nostr_sdk::TagKind::d() {
                        tag.content().map(|value| value.to_string())
                    } else {
                        None
                    }
                })?;
                if pending_d_tag != d_tag {
                    return None;
                }
                if pending.created_at_secs > created_at_secs {
                    return Some((pending.event_id.clone(), true));
                }
                if pending.created_at_secs < created_at_secs {
                    return Some((pending.event_id.clone(), false));
                }
                Some((
                    pending.event_id.clone(),
                    pending.event_id > event.id.to_string(),
                ))
            })
            .collect::<Vec<_>>();
        for (event_id, newer) in superseded_ids {
            if newer {
                return false;
            }
            self.forget_pending_relay_publish(&event_id);
        }
        true
    }
    pub(super) fn publish_local_identity_artifacts(&mut self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };

        let owner_keys = logged_in.owner_keys.clone();
        let device_keys = logged_in.device_keys.clone();
        let owner_pubkey = logged_in.owner_pubkey;
        let local_invite = self
            .protocol_engine
            .as_ref()
            .and_then(ProtocolEngine::local_invite);
        let local_app_keys = self.app_keys.get(&owner_pubkey.to_hex()).cloned();
        let local_profile = self.owner_profiles.get(&owner_pubkey.to_hex()).cloned();
        let publish_app_keys = !self.defer_owner_app_keys_publish;
        let client = logged_in.client.clone();
        let relay_urls = logged_in.relay_urls.clone();
        let tx = self.core_sender.clone();
        let update_tx = self.update_tx.clone();

        let mut background_events: Vec<(&'static str, Event)> = Vec::new();
        let mut durable_events: Vec<(&'static str, Event)> = Vec::new();

        if let (Some(keys), Some(profile)) = (owner_keys.clone(), local_profile) {
            let mut builder =
                EventBuilder::new(Kind::Metadata, build_profile_metadata_json(&profile));
            for tag_values in &profile.extra_tags {
                if let Ok(tag) = nostr::Tag::parse(tag_values.clone()) {
                    builder = builder.tag(tag);
                }
            }
            if let Ok(event) = builder.sign_with_keys(&keys) {
                background_events.push(("metadata", event));
            }
        }

        if let (true, Some(keys), Some(app_keys)) = (publish_app_keys, owner_keys, local_app_keys) {
            if let Some(ndr_app_keys) = known_app_keys_to_ndr(&app_keys) {
                if let Ok(unsigned) =
                    ndr_app_keys.get_encrypted_event_at(&keys, app_keys.created_at_secs)
                {
                    if let Ok(event) = unsigned.sign_with_keys(&keys) {
                        durable_events.push(("app-keys", event));
                    }
                }
            }
        }

        if let Some(local_invite) = local_invite {
            if let Ok(unsigned) = nostr_double_ratchet_nostr::invite_unsigned_event(&local_invite) {
                if let Ok(event) = unsigned.sign_with_keys(&device_keys) {
                    durable_events.push((LOCAL_INVITE_PUBLISH_LABEL, event));
                }
            }
        }

        for (_, event) in &background_events {
            self.remember_event(event.id.to_string());
            send_nearby_published_event(&update_tx, event);
        }
        for (label, event) in durable_events {
            self.publish_runtime_event(event, label, None);
        }

        self.runtime.spawn(async move {
            for (label, event) in background_events {
                let detail =
                    match publish_event_with_retry(&client, &relay_urls, event, label).await {
                        Ok(()) => format!("label={label} success=true"),
                        Err(error) => format!("label={label} success=false error={error}"),
                    };
                let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                    category: "publish.identity".to_string(),
                    detail,
                })));
            }
        });
    }

    pub(super) fn publish_local_app_keys(&mut self) {
        self.republish_local_identity_artifacts();
        if let Some((owner, app_keys, created_at)) = self.logged_in.as_ref().and_then(|logged_in| {
            self.app_keys
                .get(&logged_in.owner_pubkey.to_hex())
                .and_then(known_app_keys_to_ndr)
                .map(|app_keys| {
                    (
                        logged_in.owner_pubkey,
                        app_keys,
                        self.app_keys
                            .get(&logged_in.owner_pubkey.to_hex())
                            .map(|known| known.created_at_secs)
                            .unwrap_or_else(|| unix_now().get()),
                    )
                })
        }) {
            if let Some(protocol_engine) = self.protocol_engine.as_mut() {
                if let Ok(batch) =
                    protocol_engine.ingest_app_keys_snapshot(owner, app_keys, created_at)
                {
                    self.process_protocol_engine_retry_batch("publish_local_app_keys", batch);
                }
            }
        }
    }

    pub(super) fn republish_local_identity_artifacts(&mut self) {
        self.publish_local_identity_artifacts();
    }
}
