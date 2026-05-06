use super::protocol::PROTOCOL_RECONNECT_CHECK_SECS;
use super::*;

const FIRST_CONTACT_STAGE_DELAY_MS: u64 = 1_500;

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
        if self.has_seen_event(&event_id) {
            self.add_transport_channel_for_event_id(&event_id, channel);
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
            return;
        }
        self.event_transport_channels
            .insert(event_id.clone(), channel.to_string());

        let kind = event.kind.as_u16() as u32;
        if self.logged_in.is_none() {
            if self.handle_pending_link_device_response(event) {
                self.remember_event(event_id);
            }
            return;
        }

        self.push_debug_log("relay.event", format!("kind_raw={} id={event_id}", kind));
        let protocol_inputs_changed = matches!(kind, INVITE_EVENT_KIND | INVITE_RESPONSE_KIND)
            || (kind == APP_KEYS_EVENT_KIND && is_app_keys_event(&event));

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
            APP_KEYS_EVENT_KIND if is_app_keys_event(&event) => {
                self.debug_event_counters.app_keys_events += 1;
                match self.apply_app_keys_event(&event) {
                    Ok(_) => {
                        self.remember_event(event_id);
                        self.request_protocol_subscription_refresh();
                        if self.fetch_recent_protocol_state() {
                            self.state.busy.syncing_network = true;
                        }
                        self.fetch_recent_messages_for_tracked_peers(unix_now());
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
            INVITE_EVENT_KIND => {
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
            INVITE_RESPONSE_KIND => {
                self.debug_event_counters.invite_response_events += 1;
                let retry_results = self
                    .protocol_engine
                    .as_mut()
                    .map(|engine| engine.observe_invite_response_event(&event))
                    .transpose();
                match retry_results {
                    Ok(Some(results)) => {
                        self.process_protocol_engine_retry_batch("invite_response", results);
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
                if self.handle_private_chat_invite_response(&event) {
                    self.remember_event(event_id);
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
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
                self.debug_event_counters.message_events += 1;
            }
            _ => {
                self.debug_event_counters.other_events += 1;
            }
        }

        if kind == MESSAGE_EVENT_KIND {
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
            &pending.invite,
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
        effects: Vec<ProtocolEffect>,
        completions: &BTreeMap<String, (String, String)>,
    ) {
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
                ProtocolEffect::FetchRecentMessagesForOwner {
                    owner_pubkey,
                    lookback_secs,
                    reason,
                } => {
                    self.fetch_recent_messages_for_owner(
                        owner_pubkey,
                        unix_now(),
                        lookback_secs,
                        reason,
                    );
                    self.request_protocol_subscription_refresh();
                }
                ProtocolEffect::Subscribe { subid, filters } => {
                    self.apply_runtime_subscription(subid, filters);
                }
                ProtocolEffect::Unsubscribe(subid) => {
                    self.remove_runtime_subscription(subid);
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

    fn apply_runtime_subscription(&mut self, subid: String, filters: Vec<Filter>) {
        if filters.is_empty() {
            return;
        }

        self.remove_runtime_subscription_family(&subid);
        let mut changed = false;
        let mut added_authors = Vec::new();
        let multiple = filters.len() > 1;
        for (index, filter) in filters.into_iter().enumerate() {
            let indexed_subid = if multiple {
                format!("{subid}-{index}")
            } else {
                subid.clone()
            };
            changed |= self.upsert_protocol_subscription(indexed_subid.clone(), filter.clone());
            added_authors.extend(self.direct_message_subscriptions.register_subscription(
                &indexed_subid,
                serde_json::to_string(&filter).unwrap_or_default(),
            ));
        }
        added_authors.sort_by_key(|pubkey| pubkey.to_hex());
        added_authors.dedup();
        if !added_authors.is_empty() {
            self.mark_mobile_push_dirty();
            self.rebuild_state();
            self.emit_state();
        }
        for author in added_authors {
            self.fetch_recent_messages_for_author(
                author,
                unix_now(),
                NEW_MESSAGE_AUTHOR_BACKFILL_LOOKBACK_SECS,
            );
        }
        self.reconcile_protocol_subscriptions("runtime_subscribe", false);
        self.schedule_protocol_subscription_liveness_check(Duration::from_secs(30));
        self.push_debug_log(
            "runtime.subscribe",
            format!(
                "subid={subid} changed={changed} direct_authors={}",
                self.direct_message_subscriptions.tracked_authors().len()
            ),
        );
    }

    fn remove_runtime_subscription(&mut self, subid: String) {
        let removed = self.remove_runtime_subscription_family(&subid);
        let Some(client) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.client.clone())
        else {
            return;
        };
        self.runtime.spawn(async move {
            let ids = if removed.is_empty() {
                vec![subid]
            } else {
                removed
            };
            for id in ids {
                let _ = client.unsubscribe(&SubscriptionId::new(id)).await;
            }
        });
    }

    pub(super) fn remove_runtime_subscription_family(&mut self, subid: &str) -> Vec<String> {
        let previous_authors = self.direct_message_subscriptions.tracked_authors();
        let prefix = format!("{subid}-");
        let removed = self
            .protocol_subscription_runtime
            .active_subscriptions
            .keys()
            .filter(|existing| existing.as_str() == subid || existing.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        for existing in &removed {
            self.protocol_subscription_runtime
                .active_subscriptions
                .remove(existing);
            self.direct_message_subscriptions
                .unregister_subscription(existing);
        }
        if self.direct_message_subscriptions.tracked_authors() != previous_authors {
            self.mark_mobile_push_dirty();
            self.rebuild_state();
            self.emit_state();
        }
        removed
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

        let known =
            known_app_keys_from_ndr(event.pubkey, &effective_app_keys, effective_created_at);
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
