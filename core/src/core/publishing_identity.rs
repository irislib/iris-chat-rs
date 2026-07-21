use super::*;

type LabeledIdentityEvents = Vec<(&'static str, Event)>;
type LocalIdentityArtifacts = (LabeledIdentityEvents, LabeledIdentityEvents);

impl AppCore {
    fn newest_pending_app_keys_event(&self, author: PublicKey) -> Option<Event> {
        self.pending_relay_publishes
            .values()
            .filter(|pending| pending.label == "app-keys")
            .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
            .filter(|event| event.pubkey == author && is_app_keys_event(event))
            .max_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.id.cmp(&right.id))
            })
    }

    pub(super) fn build_local_identity_artifacts(&self) -> LocalIdentityArtifacts {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return (Vec::new(), Vec::new());
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
            if let Ok(unsigned) = nostr_double_ratchet::invite_unsigned_event(&local_invite) {
                if let Ok(event) = unsigned.sign_with_keys(&device_keys) {
                    durable_events.push((LOCAL_INVITE_PUBLISH_LABEL, event));
                }
            }
        }

        (background_events, durable_events)
    }

    pub(super) fn publish_local_identity_artifacts(&mut self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let client = logged_in.client.clone();
        let relay_urls = logged_in.relay_urls.clone();
        let tx = self.core_sender.clone();
        let (background_events, durable_events) = self.build_local_identity_artifacts();

        for (_, event) in &background_events {
            self.remember_event(event.id.to_string());
            self.emit_nearby_published_event(event);
        }
        for (label, event) in durable_events {
            let app_keys_author = (label == "app-keys").then_some(event.pubkey);
            if !self.publish_runtime_event(event, label, None) {
                if let Some(pending_event) =
                    app_keys_author.and_then(|author| self.newest_pending_app_keys_event(author))
                {
                    self.emit_nearby_published_event(&pending_event);
                }
            }
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

    pub(super) fn publish_local_protocol_invite(&mut self) -> bool {
        let Some((device_keys, local_invite)) = self.logged_in.as_ref().and_then(|logged_in| {
            self.protocol_engine
                .as_ref()
                .and_then(ProtocolEngine::local_invite)
                .map(|invite| (logged_in.device_keys.clone(), invite))
        }) else {
            return false;
        };
        let event = match nostr_double_ratchet::invite_unsigned_event(&local_invite)
            .and_then(|unsigned| unsigned.sign_with_keys(&device_keys).map_err(Into::into))
        {
            Ok(event) => event,
            Err(error) => {
                self.push_debug_log("publish.local_invite", error.to_string());
                return false;
            }
        };
        self.publish_runtime_event(event, LOCAL_INVITE_PUBLISH_LABEL, None)
    }

    pub(super) fn publish_local_app_keys(&mut self) {
        self.republish_local_identity_artifacts();
        self.sync_local_app_keys_to_protocol_engine("publish_local_app_keys");
    }

    pub(super) fn republish_local_identity_artifacts(&mut self) {
        self.sync_local_app_keys_if_needed();
        self.publish_local_identity_artifacts();
    }
}
