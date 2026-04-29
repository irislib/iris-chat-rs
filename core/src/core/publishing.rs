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
    ) {
        self.remember_event(event.id.to_string());
        self.emit_nearby_published_event(&event);
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };
        if relay_urls.is_empty() {
            self.push_debug_log(
                "publish.runtime",
                format!("label={label} success=false relays=0 skipped=no_servers"),
            );
            return;
        }

        let tx = self.core_sender.clone();
        let relay_count = relay_urls.len();
        self.runtime.spawn(async move {
            let result = publish_event_fire_and_forget(&client, &relay_urls, &event, label).await;
            let success = result
                .as_ref()
                .map(|relays| !relays.is_empty())
                .unwrap_or(false);
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
            if let Some((message_id, chat_id)) = completion {
                let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                    category: "publish.runtime".to_string(),
                    detail: detail.clone(),
                })));
                let _ = tx.send(CoreMsg::Internal(Box::new(
                    InternalEvent::PublishFinished {
                        message_id,
                        chat_id,
                        success,
                    },
                )));
            } else {
                let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                    category: "publish.runtime".to_string(),
                    detail,
                })));
            }
        });
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
        let client = logged_in.client.clone();
        let relay_urls = logged_in.relay_urls.clone();
        let tx = self.core_sender.clone();
        let update_tx = self.update_tx.clone();

        let mut events: Vec<(&'static str, Event)> = Vec::new();

        if let (Some(keys), Some(profile)) = (owner_keys.clone(), local_profile) {
            if let Ok(event) =
                EventBuilder::new(Kind::Metadata, build_profile_metadata_json(&profile))
                    .sign_with_keys(&keys)
            {
                events.push(("metadata", event));
            }
        }

        if let (Some(keys), Some(app_keys)) = (owner_keys, local_app_keys) {
            if let Some(ndr_app_keys) = known_app_keys_to_ndr(&app_keys) {
                if let Ok(unsigned) = ndr_app_keys.get_encrypted_event(&keys) {
                    if let Ok(event) = unsigned.sign_with_keys(&keys) {
                        events.push(("app-keys", event));
                    }
                }
            }
        }

        if let Ok(unsigned) = local_invite.get_event() {
            if let Ok(event) = unsigned.sign_with_keys(&device_keys) {
                events.push(("invite", event));
            }
        }

        for (_, event) in &events {
            self.remember_event(event.id.to_string());
            send_nearby_published_event(&update_tx, event);
        }

        self.runtime.spawn(async move {
            for (label, event) in events {
                let _ = publish_event_with_retry(&client, &relay_urls, event, label).await;
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
                logged_in
                    .ndr_runtime
                    .ingest_app_keys_snapshot(owner, app_keys, created_at);
            }
        }
        self.process_runtime_events();
    }

    pub(super) fn republish_local_identity_artifacts(&mut self) {
        self.publish_local_identity_artifacts();
    }
}
