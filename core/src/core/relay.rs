use super::*;

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
                self.apply_app_keys_event(&event);
            }
            INVITE_EVENT_KIND => {
                self.debug_event_counters.invite_events += 1;
            }
            INVITE_RESPONSE_KIND => {
                self.debug_event_counters.invite_response_events += 1;
            }
            GROUP_SENDER_KEY_MESSAGE_KIND => {
                let group_events = self
                    .logged_in
                    .as_ref()
                    .map(|logged_in| logged_in.ndr_runtime.group_handle_outer_event(&event))
                    .unwrap_or_default();
                if !group_events.is_empty() {
                    self.debug_event_counters.group_events += 1;
                    for group_event in group_events {
                        self.apply_group_decrypted_event(group_event);
                    }
                    self.remember_event(event_id);
                    self.process_runtime_events();
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
                }
            }
            MESSAGE_EVENT_KIND => {
                self.debug_event_counters.message_events += 1;
            }
            _ => {
                self.debug_event_counters.other_events += 1;
            }
        }

        if let Some(logged_in) = self.logged_in.as_ref() {
            logged_in.ndr_runtime.process_received_event(event);
        }
        let decrypted_event_ids = self.process_runtime_events();
        if kind != MESSAGE_EVENT_KIND || decrypted_event_ids.contains(&event_id) {
            self.remember_event(event_id);
        }
        if protocol_inputs_changed {
            self.setup_user_done.clear();
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
        let response = match pending
            .invite
            .process_invite_response(&event, pending.device_keys.secret_key().to_secret_bytes())
        {
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

    pub(super) fn process_runtime_events(&mut self) -> HashSet<String> {
        self.process_runtime_events_with_completions(&BTreeMap::new())
    }

    pub(super) fn process_runtime_events_with_completions(
        &mut self,
        completions: &BTreeMap<String, (String, String)>,
    ) -> HashSet<String> {
        let Some(events) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.ndr_runtime.drain_events())
        else {
            return HashSet::new();
        };

        let mut decrypted_event_ids = HashSet::new();
        for event in events {
            match event {
                SessionManagerEvent::Publish(unsigned) => {
                    if let Some(signed) = self.sign_runtime_unsigned_event(unsigned) {
                        let event_id = signed.id.to_string();
                        let completion =
                            self.runtime_publish_completion(&event_id, None, completions);
                        if !completions.is_empty() {
                            self.push_debug_log(
                                "publish.runtime.completion",
                                format!("event_id={event_id} matched={}", completion.is_some()),
                            );
                        }
                        self.publish_runtime_event(signed, "runtime", completion);
                    }
                }
                SessionManagerEvent::PublishSigned(event) => {
                    let event_id = event.id.to_string();
                    let completion = self.runtime_publish_completion(&event_id, None, completions);
                    if !completions.is_empty() {
                        self.push_debug_log(
                            "publish.runtime.completion",
                            format!("event_id={event_id} matched={}", completion.is_some()),
                        );
                    }
                    self.publish_runtime_event(event, "runtime", completion);
                }
                SessionManagerEvent::PublishSignedForInnerEvent {
                    event,
                    inner_event_id,
                    target_device_id,
                } => {
                    let event_id = event.id.to_string();
                    let completion = self.runtime_publish_completion(
                        &event_id,
                        inner_event_id.as_deref(),
                        completions,
                    );
                    if !completions.is_empty() || inner_event_id.is_some() {
                        self.push_debug_log(
                            "publish.runtime.completion",
                            format!("event_id={event_id} matched={}", completion.is_some()),
                        );
                    }
                    self.publish_runtime_event_with_metadata(
                        event,
                        "runtime",
                        completion,
                        inner_event_id,
                        target_device_id,
                    );
                }
                SessionManagerEvent::Subscribe { subid, filter_json } => {
                    self.apply_runtime_subscription(subid, filter_json);
                }
                SessionManagerEvent::Unsubscribe(subid) => {
                    self.remove_runtime_subscription(subid);
                }
                SessionManagerEvent::ReceivedEvent(event) => {
                    self.handle_relay_event_with_channel(event, "message servers");
                }
                SessionManagerEvent::DecryptedMessage {
                    sender,
                    sender_device,
                    conversation_owner,
                    content,
                    event_id,
                } => {
                    if let Some(event_id) = event_id.as_ref() {
                        decrypted_event_ids.insert(event_id.clone());
                    }
                    self.apply_decrypted_runtime_message_with_metadata(
                        sender,
                        sender_device,
                        conversation_owner,
                        content,
                        event_id,
                    );
                    // Decrypting a DM advances the double-ratchet state,
                    // so the cached mobile-push snapshot needs a refresh.
                    self.mark_mobile_push_dirty();
                }
            }
        }
        decrypted_event_ids
    }

    fn apply_runtime_subscription(&mut self, subid: String, filter_json: String) {
        let Ok(filters) = parse_runtime_subscription_filters(&filter_json) else {
            self.push_debug_log(
                "runtime.subscribe.parse",
                format!("subid={subid} invalid filter"),
            );
            return;
        };
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
            self.fetch_recent_messages_for_author(author, unix_now(), CATCH_UP_LOOKBACK_SECS);
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

    fn remove_runtime_subscription_family(&mut self, subid: &str) -> Vec<String> {
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
    pub(super) fn apply_app_keys_event(&mut self, event: &Event) {
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
            return;
        };
        let applied =
            self.apply_known_app_keys_snapshot(event.pubkey, &app_keys, event.created_at.as_secs());
        let Some((mut effective_app_keys, mut effective_created_at)) = applied else {
            if should_publish_backfilled_owner_app_keys {
                self.defer_owner_app_keys_publish = false;
            }
            return;
        };
        if should_publish_backfilled_owner_app_keys {
            if let Some((owner, device)) = self
                .logged_in
                .as_ref()
                .map(|logged_in| (logged_in.owner_pubkey, logged_in.device_keys.public_key()))
            {
                self.upsert_local_app_key_device(owner, device);
                if let Some(known) = self.app_keys.get(&owner.to_hex()) {
                    if let Some(app_keys) = known_app_keys_to_ndr(known) {
                        effective_app_keys = app_keys;
                        effective_created_at = known.created_at_secs;
                    }
                }
            }
            self.defer_owner_app_keys_publish = false;
        }
        self.migrate_verified_device_owner_threads(event.pubkey, &effective_app_keys);
        if let Some(logged_in) = self.logged_in.as_ref() {
            logged_in.ndr_runtime.ingest_app_keys_snapshot(
                event.pubkey,
                effective_app_keys,
                effective_created_at,
            );
        }
        self.mark_mobile_push_dirty();
        let _authorization_changed = self.refresh_local_authorization_state();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
        if should_publish_backfilled_owner_app_keys {
            self.publish_local_app_keys();
        }
    }
}

fn parse_runtime_subscription_filters(filter_json: &str) -> serde_json::Result<Vec<Filter>> {
    match serde_json::from_str::<Filter>(filter_json) {
        Ok(filter) => Ok(vec![filter]),
        Err(single_error) => match serde_json::from_str::<Vec<Filter>>(filter_json) {
            Ok(filters) => Ok(filters),
            Err(_) => Err(single_error),
        },
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

    #[test]
    fn runtime_subscription_parser_accepts_single_filter_and_filter_array() {
        let single =
            serde_json::to_string(&Filter::new().kind(Kind::from(MESSAGE_EVENT_KIND as u16)))
                .expect("single filter");
        assert_eq!(
            parse_runtime_subscription_filters(&single).unwrap().len(),
            1
        );

        let array = serde_json::to_string(&vec![
            Filter::new().kind(Kind::from(APP_KEYS_EVENT_KIND as u16)),
            Filter::new().kind(Kind::from(INVITE_RESPONSE_KIND as u16)),
        ])
        .expect("filter array");
        assert_eq!(parse_runtime_subscription_filters(&array).unwrap().len(), 2);
    }
}
