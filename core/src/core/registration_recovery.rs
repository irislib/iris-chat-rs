use super::*;
use nostr::RelayMessage;

impl AppCore {
    pub(super) fn recover_deferred_owner_registration(&mut self) {
        let generation = self.relay_status_watch_generation;
        if !self.defer_owner_app_keys_publish
            || self.owner_registration_lookup_generation == Some(generation)
        {
            return;
        }
        let Some(login) = self
            .logged_in
            .as_ref()
            .filter(|login| login.owner_keys.is_some())
        else {
            return;
        };
        let owner = login.owner_pubkey;
        let device = login.device_keys.public_key();
        let client = login.client.clone();
        let tx = self.core_sender.clone();
        self.owner_registration_lookup_generation = Some(generation);
        self.runtime.spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            for relay in client
                .relays()
                .await
                .into_values()
                .filter(|relay| relay.status() == RelayStatus::Connected)
            {
                jobs.spawn(async move {
                    let mut notices = relay.notifications();
                    let id = SubscriptionId::generate();
                    let filters = vec![
                        Filter::new()
                            .kind(Kind::Custom(APP_KEYS_EVENT_KIND as u16))
                            .author(owner),
                        Filter::new()
                            .kind(Kind::Custom(30078))
                            .author(owner)
                            .identifier("double-ratchet/app-keys"),
                    ];
                    if relay
                        .subscribe_with_id(id.clone(), filters, SubscribeOptions::default())
                        .await
                        .is_err()
                    {
                        return (false, Vec::new());
                    }
                    let mut events = Vec::new();
                    let mut completed = false;
                    // Keep listening after EOSE to include delayed results. A timeout
                    // without EOSE is a failed lookup, never evidence of an empty roster.
                    let _ = tokio::time::timeout(Duration::from_secs(5), async {
                        while let Ok(notice) = notices.recv().await {
                            match notice {
                                RelayNotification::Message {
                                    message:
                                        RelayMessage::Event {
                                            subscription_id,
                                            event,
                                        },
                                } if subscription_id.as_ref() == &id => {
                                    events.push(event.into_owned())
                                }
                                RelayNotification::Message {
                                    message: RelayMessage::EndOfStoredEvents(subscription_id),
                                } if subscription_id.as_ref() == &id => completed = true,
                                _ => {}
                            }
                        }
                    })
                    .await;
                    let _ = relay.unsubscribe(&id).await;
                    (completed, events)
                });
            }
            let queried = jobs.len();
            let mut completed = 0;
            let mut events = Vec::new();
            while let Some(result) = jobs.join_next().await {
                if let Ok((success, found)) = result {
                    completed += usize::from(success);
                    events.extend(found);
                }
            }
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::OwnerRegistrationLookupFinished {
                    generation,
                    owner,
                    device,
                    completed,
                    queried,
                    events,
                },
            )));
        });
    }

    pub(super) fn complete_owner_registration_lookup(
        &mut self,
        generation: u64,
        owner: PublicKey,
        device: PublicKey,
        completed: usize,
        queried: usize,
        events: Vec<Event>,
    ) {
        if generation != self.relay_status_watch_generation {
            return;
        }
        self.owner_registration_lookup_generation = None;
        if !self.defer_owner_app_keys_publish
            || !self.logged_in.as_ref().is_some_and(|login| {
                login.owner_pubkey == owner
                    && login.device_keys.public_key() == device
                    && login.owner_keys.is_some()
            })
        {
            return;
        }
        // Check every connected server, requiring corroboration when more than
        // one is configured. Offline or incomplete reads leave recovery pending.
        if queried == 0
            || completed != queried
            || completed
                < self
                    .logged_in
                    .as_ref()
                    .map_or(1, |login| login.relay_urls.len().clamp(1, 2))
        {
            self.push_debug_log(
                "registration.recovery.pending",
                format!("completed={completed} queried={queried}"),
            );
            return;
        }
        let Some((mut registration, mut observed_at)) =
            recovered_owner_registration(owner, &events)
        else {
            self.push_debug_log(
                "registration.recovery.pending",
                "conflicting registration heads",
            );
            return;
        };
        // A newer persisted roster remains authoritative over older network data.
        if let Some(known) = self
            .app_keys
            .get(&owner.to_hex())
            .filter(|known| known.created_at_secs > observed_at)
        {
            registration = known_app_keys_to_ndr(known);
            observed_at = known.created_at_secs;
        }
        registration.add_device(DeviceEntry::new(device, unix_now().get()));
        let mut known = known_app_keys_from_ndr(
            owner,
            &registration,
            next_app_keys_created_at(unix_now().get(), observed_at),
        );
        self.apply_current_device_labels_to_known_app_keys(&mut known, device);
        self.app_keys.insert(owner.to_hex(), known);
        self.defer_owner_app_keys_publish = false;
        self.publish_local_app_keys_snapshot_only("restore_registration_recovery");
        self.push_debug_log(
            "registration.recovery.queued",
            format!("checked_servers={completed}"),
        );
        self.rebuild_persist_and_emit_state();
    }
}

fn recovered_owner_registration(owner: PublicKey, events: &[Event]) -> Option<(AppKeys, u64)> {
    let valid = events.iter().filter(|event| {
        event.pubkey == owner
            && event.verify().is_ok()
            && event.created_at.as_secs() <= unix_now().get().saturating_add(300)
    });
    let mut current = valid
        .clone()
        .filter_map(|event| AppKeys::from_event(event).ok().map(|keys| (event, keys)))
        .collect::<Vec<_>>();
    current.sort_by_key(|(event, _)| event.created_at);
    if let Some((head, keys)) = current.last() {
        let devices = |keys: &AppKeys| {
            keys.get_all_devices()
                .into_iter()
                .map(|device| device.identity_pubkey)
                .collect::<std::collections::BTreeSet<_>>()
        };
        if current.iter().any(|(event, other)| {
            event.created_at == head.created_at && devices(other) != devices(keys)
        }) {
            return None;
        }
        return Some((keys.clone(), head.created_at.as_secs()));
    }
    let legacy = valid
        .filter(|event| {
            event.kind == Kind::Custom(30078)
                && event
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["d", "double-ratchet/app-keys"])
        })
        .max_by_key(|event| event.created_at);
    let devices = legacy
        .into_iter()
        .flat_map(|event| event.tags.iter())
        .filter_map(|tag| {
            let values = tag.as_slice();
            if values.first().map(String::as_str) != Some("device") {
                return None;
            }
            Some(DeviceEntry::new(
                PublicKey::parse(values.get(1)?).ok()?,
                values.get(2)?.parse().ok()?,
            ))
        })
        .collect();
    Some((
        AppKeys::new(devices),
        legacy.map_or(0, |event| event.created_at.as_secs()),
    ))
}
