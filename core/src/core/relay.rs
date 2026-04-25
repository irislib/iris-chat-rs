use super::*;

impl AppCore {
    pub(super) fn handle_relay_event(&mut self, event: Event) {
        let event_id = event.id.to_string();
        if self.has_seen_event(&event_id) || self.logged_in.is_none() {
            return;
        }

        let kind = event.kind.as_u16() as u32;
        self.push_debug_log("relay.event", format!("kind_raw={} id={event_id}", kind));

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
            MESSAGE_EVENT_KIND => {
                self.debug_event_counters.message_events += 1;
                let group_event = self
                    .logged_in
                    .as_ref()
                    .and_then(|logged_in| logged_in.ndr_runtime.group_handle_outer_event(&event));
                if let Some(group_event) = group_event {
                    self.debug_event_counters.group_events += 1;
                    self.apply_group_decrypted_event(group_event);
                    self.remember_event(event_id);
                    self.process_runtime_events();
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                    return;
                }
            }
            _ => {
                self.debug_event_counters.other_events += 1;
            }
        }

        if let Some(logged_in) = self.logged_in.as_ref() {
            logged_in
                .ndr_runtime
                .session_manager()
                .process_received_event(event);
        }
        self.remember_event(event_id);
        self.process_runtime_events();
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn process_runtime_events(&mut self) {
        self.process_runtime_events_with_completions(&BTreeMap::new());
    }

    pub(super) fn process_runtime_events_with_completions(
        &mut self,
        completions: &BTreeMap<String, (String, String)>,
    ) {
        let Some(events) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.ndr_runtime.drain_events())
        else {
            return;
        };

        for event in events {
            match event {
                SessionManagerEvent::Publish(unsigned) => {
                    if let Some(signed) = self.sign_runtime_unsigned_event(unsigned) {
                        let event_id = signed.id.to_string();
                        let completion = completions.get(&event_id).cloned();
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
                    let completion = completions.get(&event_id).cloned();
                    if !completions.is_empty() {
                        self.push_debug_log(
                            "publish.runtime.completion",
                            format!("event_id={event_id} matched={}", completion.is_some()),
                        );
                    }
                    self.publish_runtime_event(event, "runtime", completion);
                }
                SessionManagerEvent::Subscribe { subid, filter_json } => {
                    self.apply_runtime_subscription(subid, filter_json);
                }
                SessionManagerEvent::Unsubscribe(subid) => {
                    self.remove_runtime_subscription(subid);
                }
                SessionManagerEvent::ReceivedEvent(event) => {
                    self.handle_relay_event(event);
                }
                SessionManagerEvent::DecryptedMessage {
                    sender,
                    sender_device,
                    content,
                    event_id,
                } => {
                    self.apply_decrypted_runtime_message(sender, sender_device, content, event_id);
                }
            }
        }
    }

    fn apply_runtime_subscription(&mut self, subid: String, filter_json: String) {
        self.protocol_subscription_runtime
            .active_subscriptions
            .insert(subid.clone());
        let added_authors = self
            .direct_message_subscriptions
            .register_subscription(&subid, &filter_json);
        for author in added_authors {
            self.fetch_recent_messages_for_author(author, unix_now(), CATCH_UP_LOOKBACK_SECS);
        }

        let Ok(filter) = serde_json::from_str::<Filter>(&filter_json) else {
            self.push_debug_log(
                "runtime.subscribe.parse",
                format!("subid={subid} invalid filter"),
            );
            return;
        };
        let Some(client) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.client.clone())
        else {
            return;
        };
        self.runtime.spawn(async move {
            let _ = client
                .subscribe_with_id(SubscriptionId::new(subid), vec![filter], None)
                .await;
        });
    }

    fn remove_runtime_subscription(&mut self, subid: String) {
        self.protocol_subscription_runtime
            .active_subscriptions
            .remove(&subid);
        self.direct_message_subscriptions
            .unregister_subscription(&subid);
        let Some(client) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.client.clone())
        else {
            return;
        };
        self.runtime.spawn(async move {
            let _ = client.unsubscribe(SubscriptionId::new(subid)).await;
        });
    }

    fn apply_app_keys_event(&mut self, event: &Event) {
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
        let known = known_app_keys_from_ndr(event.pubkey, &app_keys, event.created_at.as_u64());
        self.app_keys.insert(event.pubkey.to_hex(), known);
        if let Some(logged_in) = self.logged_in.as_ref() {
            logged_in.ndr_runtime.ingest_app_keys_snapshot(
                event.pubkey,
                app_keys,
                event.created_at.as_u64(),
            );
        }
        if self.refresh_local_authorization_state() {
            self.rebuild_state();
            self.persist_best_effort();
            self.emit_state();
        }
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
