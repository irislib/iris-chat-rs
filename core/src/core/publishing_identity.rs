use super::*;

type LabeledIdentityEvents = Vec<(&'static str, Event)>;
type LocalIdentityArtifacts = (LabeledIdentityEvents, LabeledIdentityEvents);

impl AppCore {
    pub(super) fn signed_local_app_keys_snapshot(
        &self,
        app_keys: &AppKeys,
        created_at: u64,
        keys: &Keys,
    ) -> anyhow::Result<Event> {
        let owner = keys.public_key();
        anyhow::ensure!(
            self.logged_in
                .as_ref()
                .is_some_and(|login| login.owner_pubkey == owner),
            "Local authorization account changed."
        );
        if let Some(cached) =
            self.cached_local_fips_identity(Kind::Custom(APP_KEYS_EVENT_KIND as u16))
        {
            if cached.created_at.as_secs() == created_at
                && !private_device_labels::obsolete_private_app_keys_event(&cached)
                && AppKeys::from_event(&cached)
                    .is_ok_and(|known| roster_membership(&known) == roster_membership(app_keys))
            {
                return Ok(cached);
            }
            anyhow::ensure!(
                cached.created_at.as_secs() < created_at,
                "Local authorization revision did not advance."
            );
        }
        // The shared fact builder gives every new snapshot a random subject UUID.
        // Save its exact signed event before exposing it to any publication path;
        // repeating an unchanged snapshot must not create competing signed heads.
        let event = app_keys
            .get_event_at(owner, created_at)
            .sign_with_keys(keys)?;
        let storage = self
            .local_fips_identity_storage()
            .ok_or_else(|| anyhow::anyhow!("Local authorization storage is unavailable."))?;
        storage.put(
            &format!("appcore/nearby-identity-v1/{}", APP_KEYS_EVENT_KIND),
            serde_json::to_string(&event)?,
        )?;
        Ok(event)
    }

    pub(super) fn sync_local_app_keys_to_protocol_engine(&mut self, label: &'static str) {
        let Some((owner, app_keys, created_at, owner_keys)) =
            self.logged_in.as_ref().and_then(|logged_in| {
                let known = self.app_keys.get(&logged_in.owner_pubkey.to_hex())?;
                Some((
                    logged_in.owner_pubkey,
                    known_app_keys_to_ndr(known),
                    known.created_at_secs,
                    logged_in.owner_keys.clone(),
                ))
            })
        else {
            return;
        };

        // Keep the same durable proof used by every publication path. Linked
        // devices retain the owner signature received during approval.
        let signed = owner_keys
            .filter(|_| !self.defer_owner_app_keys_publish)
            .and_then(|keys| {
                self.signed_local_app_keys_snapshot(&app_keys, created_at, &keys)
                    .ok()
            });
        if let Some(protocol_engine) = self.protocol_engine.as_mut() {
            let result = if let Some(event) = signed {
                protocol_engine.ingest_app_keys_event(&event)
            } else {
                protocol_engine.ingest_app_keys_snapshot(owner, app_keys, created_at)
            };
            if let Ok(batch) = result {
                self.process_protocol_engine_retry_batch(label, batch);
            }
        }
    }

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
            // Republishing cached metadata is not an edit. Advancing its timestamp
            // here can overwrite a newer profile published by another client.
            let mut builder =
                EventBuilder::new(Kind::Metadata, build_profile_metadata_json(&profile))
                    .custom_created_at(Timestamp::from_secs(profile.updated_at_secs));
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
            if let Ok(event) = self.signed_local_app_keys_snapshot(
                &known_app_keys_to_ndr(&app_keys),
                app_keys.created_at_secs,
                &keys,
            ) {
                durable_events.push(("app-keys", event));
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

        let mut profile_projection_changed = false;
        for (_, event) in &background_events {
            if event.kind == Kind::Metadata && self.cache_device_sync_profile(event) {
                if let Some(profile) = self.owner_profiles.get_mut(&event.pubkey.to_hex()) {
                    let id = Some(event.id.to_hex());
                    if profile.source_event_id != id {
                        profile.source_event_id = id;
                        profile_projection_changed = true;
                    }
                }
            }
            self.remember_event(event.id.to_string());
            self.emit_nearby_published_event(event);
        }
        if profile_projection_changed {
            self.persist_best_effort();
        }
        for (label, event) in durable_events {
            let app_keys_author = (label == "app-keys").then_some(event.pubkey);
            let queued_advertisement = self
                .pending_relay_publishes
                .contains_key(&event.id.to_hex())
                .then(|| event.clone());
            if self.publish_runtime_event(event, label, None) {
                // Explicit identity publication must reach newly attached nearby
                // transports, even when its exact event survived in the outbox.
                // Generic retry effects still retain their paced, write-free path.
                if let Some(event) = queued_advertisement {
                    self.emit_nearby_published_event(&event);
                }
            } else if let Some(pending_event) =
                app_keys_author.and_then(|author| self.newest_pending_app_keys_event(author))
            {
                self.emit_nearby_published_event(&pending_event);
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

fn roster_membership(app_keys: &AppKeys) -> BTreeMap<PublicKey, u64> {
    app_keys
        .get_all_devices()
        .into_iter()
        .map(|device| (device.identity_pubkey, device.created_at))
        .collect()
}
