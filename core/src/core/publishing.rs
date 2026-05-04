use super::*;

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
        self.publish_runtime_event_with_metadata(event, label, completion, None, None)
    }

    pub(super) fn publish_runtime_event_with_metadata(
        &mut self,
        event: Event,
        label: &'static str,
        completion: Option<(String, String)>,
        inner_event_id: Option<String>,
        target_device_id: Option<String>,
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
        let stored = self.remember_pending_relay_publish(
            &event,
            label,
            completion.clone(),
            inner_event_id,
            target_device_id,
        );
        if !stored {
            return false;
        }
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return false;
        };
        if relay_urls.is_empty() {
            let (message_id, chat_id) = completion
                .map(|(message_id, chat_id)| (Some(message_id), Some(chat_id)))
                .unwrap_or((None, None));
            self.handle_relay_publish_finished(
                event_id,
                message_id,
                chat_id,
                false,
                Vec::new(),
                format!("label={label} success=false relays=0 skipped=no_servers"),
            );
            return true;
        }

        self.spawn_relay_publish_attempt(event, label.to_string(), completion, client, relay_urls);
        true
    }

    fn spawn_relay_publish_attempt(
        &mut self,
        event: Event,
        label: String,
        completion: Option<(String, String)>,
        client: Client,
        relay_urls: Vec<RelayUrl>,
    ) {
        let event_id = event.id.to_string();
        if !self.pending_relay_publish_inflight.insert(event_id.clone()) {
            return;
        }
        let tx = self.core_sender.clone();
        let relay_count = relay_urls.len();
        self.runtime.spawn(async move {
            let result = publish_event_fire_and_forget(&client, &relay_urls, &event, &label).await;
            let success = result
                .as_ref()
                .map(|relays| !relays.is_empty())
                .unwrap_or(false);
            let relay_urls = result
                .as_ref()
                .map(|relays| relays.clone())
                .unwrap_or_default();
            let detail = match &result {
                Ok(relays) => {
                    format!(
                        "label={label} success=true relays={relay_count} queued_relays={}",
                        relays.join(",")
                    )
                }
                Err(error) => {
                    format!("label={label} success=false relays={relay_count} error={error}")
                }
            };
            let (message_id, chat_id) = completion
                .map(|(message_id, chat_id)| (Some(message_id), Some(chat_id)))
                .unwrap_or((None, None));
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::RelayPublishFinished {
                    event_id,
                    message_id,
                    chat_id,
                    success,
                    relay_urls,
                    detail,
                },
            )));
        });
    }

    fn remember_pending_relay_publish(
        &mut self,
        event: &Event,
        label: &str,
        completion: Option<(String, String)>,
        inner_event_id: Option<String>,
        target_device_id: Option<String>,
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
        let (message_id, chat_id) = completion
            .map(|(message_id, chat_id)| (Some(message_id), Some(chat_id)))
            .unwrap_or((None, None));
        let pending = PendingRelayPublish {
            owner_pubkey_hex,
            event_id: event.id.to_string(),
            label: label.to_string(),
            event_json,
            inner_event_id,
            target_device_id,
            message_id,
            chat_id,
            created_at_secs: event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        };
        if !self.prune_or_skip_superseded_app_keys_publish(event) {
            return false;
        }
        if let Err(error) = self.app_store.upsert_pending_relay_publish(&pending) {
            self.push_debug_log("publish.runtime.queue", format!("store_failed={error}"));
            return false;
        }
        if let (Some(message_id), Some(chat_id)) =
            (pending.message_id.as_deref(), pending.chat_id.as_deref())
        {
            self.record_message_outer_event(
                chat_id,
                message_id,
                &pending.event_id,
                pending.target_device_id.as_deref(),
            );
        }
        self.pending_relay_publishes
            .insert(pending.event_id.clone(), pending);
        if let Some(pending) = self.pending_relay_publishes.get(&event.id.to_string()) {
            if let (Some(message_id), Some(chat_id)) =
                (pending.message_id.clone(), pending.chat_id.clone())
            {
                self.sync_message_delivery_trace(&chat_id, &message_id);
            }
        }
        true
    }

    pub(super) fn retry_pending_relay_publishes(&mut self, reason: &'static str) {
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

        let pending = self
            .pending_relay_publishes
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut started = 0usize;
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
            started += 1;
            self.spawn_relay_publish_attempt(
                event,
                pending.label.clone(),
                pending.message_id.clone().zip(pending.chat_id.clone()),
                client.clone(),
                relay_urls.clone(),
            );
        }
        if started > 0 {
            self.push_debug_log(
                "publish.runtime.retry",
                format!(
                    "reason={reason} started={started} pending={}",
                    self.pending_relay_publishes.len()
                ),
            );
        }
    }

    pub(super) fn handle_relay_publish_finished(
        &mut self,
        event_id: String,
        message_id: Option<String>,
        chat_id: Option<String>,
        success: bool,
        relay_urls: Vec<String>,
        detail: String,
    ) {
        self.pending_relay_publish_inflight.remove(&event_id);
        self.push_debug_log("publish.runtime", detail.clone());
        if success {
            self.forget_pending_relay_publish(&event_id);
        } else if let Some(pending) = self.pending_relay_publishes.get_mut(&event_id) {
            pending.attempt_count = pending.attempt_count.saturating_add(1);
            pending.last_error = Some(detail.clone());
            if let Err(error) = self.app_store.upsert_pending_relay_publish(pending) {
                self.push_debug_log("publish.runtime.queue", format!("update_failed={error}"));
            }
        }
        if let (Some(message_id), Some(chat_id)) = (message_id, chat_id) {
            for relay_url in &relay_urls {
                self.add_message_transport_channel(
                    &chat_id,
                    &message_id,
                    &format!("message server: {relay_url}"),
                );
            }
            let delivery = if success {
                DeliveryState::Sent
            } else {
                DeliveryState::Queued
            };
            self.update_message_delivery(&chat_id, &message_id, delivery);
            self.sync_message_delivery_trace(&chat_id, &message_id);
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn forget_pending_relay_publish(&mut self, event_id: &str) {
        self.pending_relay_publishes.remove(event_id);
        if let Err(error) = self.app_store.delete_pending_relay_publish(event_id) {
            self.push_debug_log("publish.runtime.queue", format!("delete_failed={error}"));
        }
    }

    fn prune_or_skip_superseded_app_keys_publish(&mut self, event: &Event) -> bool {
        if !is_app_keys_event(event) {
            return true;
        }

        let current_event_id = event.id.to_string();
        let current_created_at = event.created_at.as_secs();
        let mut superseded_by_newer = None;
        let mut stale_event_ids = Vec::new();

        for pending in self.pending_relay_publishes.values() {
            if pending.event_id == current_event_id || pending.label != "app-keys" {
                continue;
            }
            let Ok(pending_event) = serde_json::from_str::<Event>(&pending.event_json) else {
                continue;
            };
            if !is_app_keys_event(&pending_event) || pending_event.pubkey != event.pubkey {
                continue;
            }
            if pending_event.created_at.as_secs() > current_created_at {
                superseded_by_newer = Some(pending.event_id.clone());
            } else {
                stale_event_ids.push(pending.event_id.clone());
            }
        }

        if let Some(newer_event_id) = superseded_by_newer {
            self.push_debug_log(
                "publish.runtime.queue",
                format!(
                    "label=app-keys skipped=superseded_by_newer pending_event_id={newer_event_id}"
                ),
            );
            return false;
        }

        for stale_event_id in stale_event_ids {
            self.push_debug_log(
                "publish.runtime.queue",
                format!("label=app-keys dropped=superseded event_id={stale_event_id}"),
            );
            self.forget_pending_relay_publish(&stale_event_id);
        }

        true
    }

    pub(super) fn sign_runtime_unsigned_event(&self, event: UnsignedEvent) -> Option<Event> {
        let logged_in = self.logged_in.as_ref()?;
        if event.pubkey == logged_in.device_keys.public_key() {
            return event.sign_with_keys(&logged_in.device_keys).ok();
        }
        if let Some(owner_keys) = logged_in.owner_keys.as_ref() {
            if event.pubkey == owner_keys.public_key() {
                return event.sign_with_keys(owner_keys).ok();
            }
        }
        None
    }

    pub(super) fn publish_local_identity_artifacts(&mut self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };

        let owner_keys = logged_in.owner_keys.clone();
        let device_keys = logged_in.device_keys.clone();
        let owner_pubkey = logged_in.owner_pubkey;
        let local_invite = logged_in.local_invite.clone();
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
            if let Ok(event) =
                EventBuilder::new(Kind::Metadata, build_profile_metadata_json(&profile))
                    .sign_with_keys(&keys)
            {
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

        if let Ok(unsigned) = nostr_double_ratchet_nostr::invite_unsigned_event(&local_invite) {
            if let Ok(event) = unsigned.sign_with_keys(&device_keys) {
                durable_events.push(("invite", event));
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
            let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::SyncComplete)));
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
            if let Some(logged_in) = self.logged_in.as_ref() {
                if let Ok(effects) = logged_in
                    .ndr_runtime
                    .ingest_app_keys_snapshot(owner, app_keys, created_at)
                {
                    self.process_runtime_effects(effects);
                }
            }
        }
    }

    pub(super) fn republish_local_identity_artifacts(&mut self) {
        self.publish_local_identity_artifacts();
    }
}
