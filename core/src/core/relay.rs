use super::protocol::PROTOCOL_RECONNECT_CHECK_SECS;
use super::*;

#[cfg(not(test))]
const FIRST_CONTACT_STAGE_DELAY_MS: u64 = 1_500;
#[cfg(test)]
const FIRST_CONTACT_STAGE_DELAY_MS: u64 = 25;

fn coalesce_protocol_fetch_effects(effects: &mut Vec<ProtocolEffect>) {
    let mut seen = HashSet::new();
    effects.retain(|effect| match effect {
        ProtocolEffect::FetchProtocolState { filters, reason } => {
            seen.insert(format!("{reason}:{filters:?}"))
        }
        _ => true,
    });
}

impl AppCore {
    pub(super) fn runtime_publish_completion(
        &self,
        event_id: &str,
        inner_event_id: Option<&str>,
        completions: &BTreeMap<String, (String, String)>,
    ) -> Option<(String, String)> {
        completions.get(event_id).cloned().or_else(|| {
            inner_event_id.and_then(|message_id| {
                self.find_message_chat_id(message_id)
                    .map(|chat_id| (message_id.to_string(), chat_id))
            })
        })
    }

    pub(super) fn handle_relay_event(&mut self, event: Event) {
        self.handle_relay_event_with_channel(event, "message servers");
    }

    pub(super) fn handle_relay_event_with_channel(&mut self, event: Event, channel: &str) {
        let event_id = event.id.to_string();
        let kind = event.kind.as_u16() as u32;
        let is_app_keys_protocol_event = kind == APP_KEYS_EVENT_KIND && is_app_keys_event(&event);
        let is_invite_protocol_event = is_protocol_invite_event(&event);
        let is_invite_response_protocol_event = kind == INVITE_RESPONSE_KIND;
        if self.has_seen_event(&event_id) {
            // Only persist + rebuild + emit when the transport-channel
            // set actually grew. Without this guard, every mirrored
            // relay re-delivery of an already-seen event burns a full
            // snapshot serialize + state rebuild — measurably hot on
            // accounts subscribed to a handful of relays.
            if self.add_transport_channel_for_event_id(&event_id, channel) {
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
            }
            if !self.should_replay_seen_protocol_event(&event, is_invite_protocol_event) {
                return;
            }
        }
        self.event_transport_channels
            .insert(event_id.clone(), channel.to_string());

        if self.logged_in.is_none() {
            if self.handle_pending_link_device_response(event) {
                self.remember_event(event_id);
            }
            return;
        }

        self.push_debug_log("relay.event", format!("kind_raw={} id={event_id}", kind));
        let protocol_inputs_changed = is_app_keys_protocol_event
            || is_invite_protocol_event
            || is_invite_response_protocol_event;

        if kind == 0 {
            if self.apply_profile_metadata_event(&event) {
                self.remember_event(event_id);
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
                return;
            }
            self.remember_event(event_id);
            return;
        }

        match kind {
            APP_KEYS_EVENT_KIND if is_app_keys_protocol_event => {
                self.debug_event_counters.app_keys_events += 1;
                match self.apply_app_keys_event(&event) {
                    Ok(_) => {
                        self.remember_event(event_id);
                        self.request_protocol_subscription_refresh();
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                    }
                    Err(error) => {
                        self.push_debug_log("appcore.protocol.app_keys.error", error.to_string());
                        self.rebuild_state();
                        self.emit_state();
                    }
                }
                return;
            }
            INVITE_EVENT_KIND if is_invite_protocol_event => {
                self.debug_event_counters.invite_events += 1;
                let retry_results = self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.observe_invite_event(&event))
                    .transpose();
                match retry_results {
                    Ok(Some(results)) => {
                        self.process_protocol_engine_retry_batch("invite_event", results);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.push_debug_log("appcore.protocol.invite.error", error.to_string());
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                }
            }
            INVITE_EVENT_KIND => {
                self.remember_event(event_id);
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
                return;
            }
            INVITE_RESPONSE_KIND => {
                self.debug_event_counters.invite_response_events += 1;
                if self.handle_private_chat_invite_response(&event) {
                    self.remember_event(event_id);
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
                }
                let retry_results = self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.observe_invite_response_event(&event))
                    .transpose();
                match retry_results {
                    Ok(Some(results)) => {
                        self.process_protocol_engine_retry_batch("invite_response", results);
                        self.refresh_local_authorization_state();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.push_debug_log(
                            "appcore.protocol.invite_response.error",
                            error.to_string(),
                        );
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                }
            }
            MESSAGE_EVENT_KIND => {
                let group_result = match self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.process_group_outer_event(&event))
                {
                    Some(Ok(group_result)) => group_result,
                    Some(Err(error)) => {
                        self.push_debug_log(
                            "appcore.protocol.group.outer.error",
                            error.to_string(),
                        );
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                    None => Default::default(),
                };
                if group_result.consumed
                    || !group_result.events.is_empty()
                    || !group_result.effects.is_empty()
                    || !group_result.queued_targets.is_empty()
                {
                    self.debug_event_counters.group_events += 1;
                    let should_remember_group_event = group_result.consumed
                        || !group_result.events.is_empty()
                        || !group_result.effects.is_empty();
                    if !group_result.queued_targets.is_empty() {
                        self.handle_queued_protocol_targets(
                            "group.outer",
                            &group_result.queued_targets,
                        );
                    }
                    for group_event in group_result.events {
                        self.apply_group_decrypted_event(group_event);
                    }
                    if !group_result.effects.is_empty() {
                        self.process_protocol_engine_effects_with_completions(
                            group_result.effects,
                            &BTreeMap::new(),
                        );
                    }
                    if should_remember_group_event {
                        self.remember_event(event_id);
                    }
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
                }
                if self
                    .protocol_engine
                    .as_ref()
                    .is_some_and(|engine| !engine.is_known_message_author(event.pubkey))
                {
                    self.push_debug_log(
                        "appcore.protocol.message.ignored",
                        "unknown message author",
                    );
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
                }
                self.debug_event_counters.message_events += 1;
            }
            _ => {
                self.debug_event_counters.other_events += 1;
            }
        }

        if kind == MESSAGE_EVENT_KIND {
            if self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.has_pending_inbound_direct_event_id(&event_id))
            {
                self.push_debug_log(
                    "appcore.protocol.message.pending_replay",
                    "already stored as pending inbound",
                );
                self.request_protocol_subscription_refresh();
                self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                    PROTOCOL_RECONNECT_CHECK_SECS,
                ));
                self.persist_best_effort();
                self.rebuild_state();
                self.emit_state();
                return;
            }
            if let Some(protocol_engine) = self.protocol_engine.as_mut() {
                match protocol_engine.process_direct_message_event(&event) {
                    Ok(Some(decrypted)) => {
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
                        self.mark_mobile_push_dirty();
                        self.request_protocol_subscription_refresh();
                        self.retry_protocol_engine_pending_outbound("direct_message");
                        self.persist_best_effort();
                        self.rebuild_state();
                        self.emit_state();
                        return;
                    }
                    Ok(None) => {
                        self.push_debug_log(
                            "appcore.protocol.message.pending",
                            "sender/session unresolved",
                        );
                        let (queued_targets, effects) = self
                            .protocol_engine
                            .as_ref()
                            .map(|engine| {
                                engine.queued_protocol_backfill_effects(
                                    NdrUnixSeconds(unix_now().get()),
                                    "direct_message.pending",
                                )
                            })
                            .unwrap_or_default();
                        self.process_protocol_engine_effects_with_completions(
                            effects,
                            &BTreeMap::new(),
                        );
                        if queued_targets.is_empty() {
                            self.request_protocol_subscription_refresh();
                            self.schedule_protocol_subscription_liveness_check(
                                Duration::from_secs(PROTOCOL_RECONNECT_CHECK_SECS),
                            );
                        } else {
                            self.handle_queued_protocol_targets(
                                "direct_message.pending",
                                &queued_targets,
                            );
                        }
                        if queued_targets.is_empty() && self.fetch_recent_protocol_state() {
                            self.state.busy.syncing_network = true;
                        }
                    }
                    Err(error) => {
                        self.push_debug_log("appcore.protocol.message.error", error.to_string());
                    }
                }
            }
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
            return;
        }
        self.remember_event(event_id);
        if protocol_inputs_changed {
            self.request_protocol_subscription_refresh();
            if self.fetch_recent_protocol_state() {
                self.state.busy.syncing_network = true;
            }
            self.schedule_tracked_peer_catch_up(Duration::from_secs(2));
        }
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    fn handle_pending_link_device_response(&mut self, event: Event) -> bool {
        if event.kind.as_u16() as u32 != INVITE_RESPONSE_KIND {
            return false;
        }
        let Some(pending) = self.pending_linked_device.as_ref() else {
            return false;
        };

        self.debug_event_counters.invite_response_events += 1;
        let event_id = event.id.to_string();
        let response = match nostr_double_ratchet_nostr::process_invite_response_event(
            &pending.pairing_invite,
            &event,
            pending.device_keys.secret_key().to_secret_bytes(),
        ) {
            Ok(Some(response)) => response,
            Ok(None) => return false,
            Err(error) => {
                self.push_debug_log(
                    "session.link_response.error",
                    format!("event_id={event_id} error={error}"),
                );
                return false;
            }
        };

        let owner_pubkey = response
            .owner_public_key
            .unwrap_or(response.invitee_identity);
        let peer_device_id = response
            .device_id
            .clone()
            .unwrap_or_else(|| response.invitee_identity.to_hex());
        let session_state = response.session.state;
        let device_keys = pending.device_keys.clone();
        self.push_debug_log(
            "session.link_response",
            format!(
                "event_id={event_id} owner={} peer_device={peer_device_id}",
                owner_pubkey.to_hex()
            ),
        );

        if let Err(error) = self.complete_pending_linked_device(
            owner_pubkey,
            peer_device_id,
            session_state,
            device_keys,
        ) {
            self.state.toast = Some(error.to_string());
            self.rebuild_state();
            self.emit_state();
        }
        true
    }

    pub(super) fn process_protocol_engine_effects_with_completions(
        &mut self,
        mut effects: Vec<ProtocolEffect>,
        completions: &BTreeMap<String, (String, String)>,
    ) {
        coalesce_protocol_fetch_effects(&mut effects);
        for effect in effects {
            match effect {
                ProtocolEffect::PublishUnsigned(unsigned) => {
                    if let Some(signed) = self.sign_runtime_unsigned_event(unsigned) {
                        let event_id = signed.id.to_string();
                        let completion =
                            self.runtime_publish_completion(&event_id, None, completions);
                        self.publish_runtime_event(signed, "appcore-protocol", completion);
                    }
                }
                ProtocolEffect::PublishSigned(event) => {
                    let event_id = event.id.to_string();
                    let completion = self.runtime_publish_completion(&event_id, None, completions);
                    self.publish_runtime_event(event, APPCORE_PROTOCOL_LABEL, completion);
                }
                ProtocolEffect::PublishSignedForInnerEvent {
                    event,
                    inner_event_id,
                    target_owner_pubkey_hex,
                    target_device_id,
                } => {
                    let event_id = event.id.to_string();
                    let completion = self.runtime_publish_completion(
                        &event_id,
                        inner_event_id.as_deref(),
                        completions,
                    );
                    self.publish_runtime_event_with_metadata(
                        event,
                        APPCORE_PROTOCOL_LABEL,
                        completion,
                        inner_event_id,
                        target_owner_pubkey_hex,
                        target_device_id,
                    );
                }
                ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => {
                    for publish in bootstrap {
                        self.publish_protocol_event(publish, completions);
                    }
                    let mut queued_payloads = 0usize;
                    for publish in payload {
                        if self.queue_protocol_event_for_delayed_publish(publish, completions) {
                            queued_payloads = queued_payloads.saturating_add(1);
                        }
                    }
                    if queued_payloads > 0 {
                        self.push_debug_log(
                            "appcore.protocol.first_contact_staged",
                            format!("queued_payloads={queued_payloads}"),
                        );
                        self.schedule_first_contact_payload_publish();
                    }
                }
                ProtocolEffect::Subscribe { subid, filters } => {
                    self.push_debug_log(
                        "appcore.protocol.subscribe_ignored",
                        format!(
                            "subid={subid} filters={} using_appcore_plan=true",
                            filters.len()
                        ),
                    );
                    self.request_protocol_subscription_refresh();
                }
                ProtocolEffect::Unsubscribe(subid) => {
                    self.push_debug_log(
                        "appcore.protocol.unsubscribe_ignored",
                        format!("subid={subid} using_appcore_plan=true"),
                    );
                    self.request_protocol_subscription_refresh();
                }
                ProtocolEffect::FetchBackfill => {
                    self.fetch_recent_protocol_state();
                }
                ProtocolEffect::FetchProtocolState { filters, reason } => {
                    self.fetch_protocol_state_for_filters(filters, reason);
                }
                ProtocolEffect::EmitDecrypted {
                    sender,
                    sender_device,
                    conversation_owner,
                    content,
                    event_id,
                } => {
                    self.apply_decrypted_runtime_message_with_metadata(
                        sender,
                        sender_device,
                        conversation_owner,
                        content,
                        event_id,
                    );
                    self.mark_mobile_push_dirty();
                }
            }
        }
    }

    fn publish_protocol_event(
        &mut self,
        publish: ProtocolPublishEvent,
        completions: &BTreeMap<String, (String, String)>,
    ) {
        let event_id = publish.event.id.to_string();
        let completion = self.runtime_publish_completion(
            &event_id,
            publish.inner_event_id.as_deref(),
            completions,
        );
        self.publish_runtime_event_with_metadata(
            publish.event,
            APPCORE_PROTOCOL_BOOTSTRAP_LABEL,
            completion,
            publish.inner_event_id,
            publish.target_owner_pubkey_hex,
            publish.target_device_id,
        );
    }

    fn queue_protocol_event_for_delayed_publish(
        &mut self,
        publish: ProtocolPublishEvent,
        completions: &BTreeMap<String, (String, String)>,
    ) -> bool {
        let event_id = publish.event.id.to_string();
        let completion = self.runtime_publish_completion(
            &event_id,
            publish.inner_event_id.as_deref(),
            completions,
        );
        self.queue_runtime_event_for_delayed_publish(
            publish.event,
            APPCORE_PROTOCOL_FIRST_CONTACT_LABEL,
            completion,
            publish.inner_event_id,
            publish.target_owner_pubkey_hex,
            publish.target_device_id,
        )
    }

    pub(super) fn schedule_first_contact_payload_publish(&self) {
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_millis(FIRST_CONTACT_STAGE_DELAY_MS)).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::RetryPendingRelayPublishes {
                    reason: "first_contact_stage".to_string(),
                },
            )));
        });
    }

    pub(super) fn ack_pending_decrypted_deliveries_after_app_persist(&mut self) {
        if let Some(protocol_engine) = self.protocol_engine.as_mut() {
            if let Err(error) = protocol_engine.ack_pending_decrypted_deliveries() {
                self.push_debug_log("appcore.protocol.decrypted_ack.error", error.to_string());
            }
        }
        self.pending_decrypted_delivery_acks.clear();
    }

    /// Process an app-keys event (kind 30078) — adds/removes devices
    /// for an owner. The mobile-push snapshot indexes by tracked owner
    /// + device, so any change there invalidates the cache.
    pub(super) fn apply_app_keys_event(&mut self, event: &Event) -> anyhow::Result<bool> {
        let should_publish_backfilled_owner_app_keys =
            self.logged_in.as_ref().is_some_and(|logged_in| {
                self.defer_owner_app_keys_publish
                    && logged_in.owner_keys.is_some()
                    && logged_in.owner_pubkey == event.pubkey
            });
        let app_keys = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| {
                logged_in
                    .owner_keys
                    .as_ref()
                    .filter(|keys| keys.public_key() == event.pubkey)
                    .and_then(|keys| AppKeys::from_event_with_labels(event, keys).ok())
            })
            .or_else(|| AppKeys::from_event(event).ok());
        let Some(app_keys) = app_keys else {
            return Ok(false);
        };

        let owner_hex = event.pubkey.to_hex();
        let current = self.app_keys.get(&owner_hex).cloned();
        let current_app_keys = current.as_ref().and_then(known_app_keys_to_ndr);
        let current_created_at = current
            .as_ref()
            .map(|known| known.created_at_secs)
            .unwrap_or_default();
        let required_device = self
            .logged_in
            .as_ref()
            .filter(|logged_in| {
                self.defer_owner_app_keys_publish
                    && logged_in.owner_keys.is_some()
                    && logged_in.owner_pubkey == event.pubkey
            })
            .map(|logged_in| {
                DeviceEntry::new(logged_in.device_keys.public_key(), unix_now().get())
            });
        let applied = apply_app_keys_snapshot_with_required_device(
            current_app_keys.as_ref(),
            current_created_at,
            &app_keys,
            event.created_at.as_secs(),
            required_device,
        );
        let effective_app_keys = applied.app_keys;
        let effective_created_at = applied.created_at;

        let protocol_retry_batch = if let Some(protocol_engine) = self.protocol_engine.as_mut() {
            protocol_engine.ingest_app_keys_snapshot(
                event.pubkey,
                effective_app_keys.clone(),
                effective_created_at,
            )?
        } else {
            ProtocolRetryBatch::default()
        };

        let mut known =
            known_app_keys_from_ndr(event.pubkey, &effective_app_keys, effective_created_at);
        if should_publish_backfilled_owner_app_keys {
            if let Some(device_pubkey) = self
                .logged_in
                .as_ref()
                .filter(|logged_in| logged_in.owner_pubkey == event.pubkey)
                .map(|logged_in| logged_in.device_keys.public_key())
            {
                self.apply_current_device_labels_to_known_app_keys(&mut known, device_pubkey);
            }
        }
        if current.as_ref() != Some(&known) {
            self.app_keys.insert(owner_hex, known);
        }
        if should_publish_backfilled_owner_app_keys {
            self.defer_owner_app_keys_publish = false;
        }
        self.migrate_verified_device_owner_threads(event.pubkey, &effective_app_keys);
        self.mark_mobile_push_dirty();
        let _authorization_changed = self.refresh_local_authorization_state();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
        self.process_protocol_engine_retry_batch("app_keys", protocol_retry_batch);
        if should_publish_backfilled_owner_app_keys {
            self.publish_local_app_keys();
        }
        Ok(true)
    }
}

impl AppCore {
    fn should_replay_seen_protocol_event(
        &self,
        event: &Event,
        is_invite_protocol_event: bool,
    ) -> bool {
        is_invite_protocol_event
            && self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.has_queued_invite_author(event.pubkey))
    }
}

fn is_protocol_invite_event(event: &Event) -> bool {
    event.kind.as_u16() as u32 == INVITE_EVENT_KIND
        && event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(|value| value.as_str()) == Some("d")
                && values
                    .get(1)
                    .is_some_and(|value| value.starts_with(NDR_INVITES_D_TAG_PREFIX))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Tag;

    #[test]
    fn parses_message_expiration_tag_seconds_and_milliseconds() {
        let tags = vec![Tag::parse(["expiration", "1704067260123"]).expect("expiration tag")];

        assert_eq!(message_expiration_from_tags(&tags), Some(1_704_067_260));
    }
}
