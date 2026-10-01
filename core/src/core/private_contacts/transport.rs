use super::*;

impl AppCore {
    pub(in crate::core) fn start_private_contact_sync(&mut self) {
        if let Err(error) = self.private_contact_state() {
            self.push_debug_log("private_contacts.restore_failed", error.to_string());
            return;
        }
        self.private_contacts.generation = self.private_contacts.generation.wrapping_add(1);
        self.recover_private_contact_history();
        self.send_private_contact_control(
            serde_json::json!({"type":"private-contact-sync", "v":1, "request":true}),
        );
        self.private_contact_sync_tick(self.private_contacts.generation);
    }

    pub(in crate::core) fn recover_private_contact_history(&mut self) {
        self.private_contacts.stop_network();
        if let Some(login) = self
            .logged_in
            .as_ref()
            .filter(|login| login.owner_keys.is_some())
        {
            let client = login.client.clone();
            let relays = login.relay_urls.clone();
            let filter: anyhow::Result<Filter> =
                private_contact_sync_filter(&login.owner_pubkey.to_hex())
                    .and_then(|value| Ok(serde_json::from_value(value)?));
            let Ok(filter) = filter else {
                return;
            };
            let tx = self.core_sender.clone();
            self.private_contacts.history = Some(self.runtime.spawn(async move {
                ensure_session_relays_configured(&client, &relays).await;
                let _ = client
                    .subscribe_with_id(
                        SubscriptionId::new("iris-private-contacts-v1"),
                        filter.clone(),
                        None,
                    )
                    .await;
                connect_client_with_timeout(&client, Duration::from_secs(5)).await;
                history::recover(&client, filter, tx).await;
            }));
        }
    }

    pub(in crate::core) fn private_contact_sync_tick(&mut self, generation: u64) {
        if generation != self.private_contacts.generation || self.logged_in.is_none() {
            return;
        }
        self.flush_private_contact_events();
        let delay = self
            .private_contacts
            .retry_at
            .map(|at| at.saturating_sub(unix_now().get()).clamp(1, 30))
            .unwrap_or(30);
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_secs(delay)).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::PrivateContactSyncTick { generation },
            )));
        });
    }

    pub(super) fn kick_private_contact_sync(&mut self) {
        self.private_contacts.generation = self.private_contacts.generation.wrapping_add(1);
        self.private_contact_sync_tick(self.private_contacts.generation);
    }

    pub(super) fn flush_private_contact_events(&mut self) {
        self.private_contacts.retry_at = None;
        let Some(keys) = self
            .logged_in
            .as_ref()
            .and_then(|login| login.owner_keys.clone())
        else {
            return;
        };
        let contacts = self
            .private_contacts
            .state
            .as_ref()
            .map(|state| {
                pending_private_contacts(state)
                    .iter()
                    .map(|record| record.document.contact.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for contact in contacts {
            let result = (|| -> anyhow::Result<()> {
                let Some(state) = self.private_contacts.state.as_ref() else {
                    return Ok(());
                };
                let prepared =
                    prepare_private_contact_event(state, &contact, &keys, unix_now().get())?;
                if let Some(at) = prepared.retry_at {
                    self.private_contacts.retry_at = Some(
                        self.private_contacts
                            .retry_at
                            .map_or(at, |current| current.min(at)),
                    );
                }
                if prepared.state != *state {
                    self.commit_private_contacts(prepared.state)?;
                }
                if let Some(event) = prepared.event {
                    if !self
                        .pending_relay_publishes
                        .contains_key(&event.id.to_hex())
                    {
                        anyhow::ensure!(
                            self.publish_runtime_event(event, "private-contact-sync", None),
                            "could not queue private contact event"
                        );
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.push_debug_log("private_contacts.publish_failed", error.to_string());
            }
        }
    }

    pub(in crate::core) fn acknowledge_private_contact_publish(&mut self, event_id: &str) {
        let Some(state) = self.private_contacts.state.as_ref() else {
            return;
        };
        let Some(contact) = state
            .records
            .iter()
            .find(|(_, record)| {
                record
                    .event
                    .as_ref()
                    .is_some_and(|event| event.id.to_hex() == event_id)
            })
            .map(|(contact, _)| contact.clone())
        else {
            return;
        };
        let next = acknowledge_private_contact_event(state, &contact, event_id);
        if next != *state {
            if let Err(error) = self.commit_private_contacts(next) {
                self.push_debug_log("private_contacts.ack_failed", error.to_string());
            }
        }
    }

    pub(in crate::core) fn receive_private_contact_event(&mut self, event: &Event) -> bool {
        if event.kind != Kind::from(PRIVATE_CONTACT_SYNC_KIND)
            || !event
                .tags
                .iter()
                .any(|tag| tag.as_slice() == ["t", PRIVATE_CONTACT_SYNC_NAMESPACE])
        {
            return false;
        }
        let Some(login) = self.logged_in.as_ref() else {
            return true;
        };
        if event.pubkey != login.owner_pubkey {
            return true;
        }
        let Some(keys) = login.owner_keys.clone() else {
            return true;
        };
        let result = (|| -> anyhow::Result<()> {
            let state = self.private_contact_state()?;
            if state.received_event_ids.contains(&event.id.to_hex()) {
                return Ok(());
            }
            let document = open_private_contact_event(event, &state.owner, &keys)?;
            let merged = merge_private_contact_document(&state, &document)?;
            let changed = merged.contacts != state.contacts;
            let next = remember_private_contact_event(&merged, &event.id.to_hex())?;
            self.commit_private_contacts(next)?;
            if changed {
                self.send_private_contact_document(&document);
                self.broadcast_device_sync_snapshot();
                self.rebuild_persist_and_emit_state();
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.push_debug_log("private_contacts.receive_failed", error.to_string());
        }
        true
    }

    fn send_private_contact_control(&mut self, value: serde_json::Value) {
        let Some(owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return;
        };
        let unsigned = EventBuilder::new(
            Kind::from(PRIVATE_CONTACT_CONTROL_KIND as u16),
            value.to_string(),
        )
        .tag(nostr::Tag::public_key(owner))
        .build(owner);
        self.send_protocol_engine_unsigned_event_to_local_siblings(
            owner,
            &owner.to_hex(),
            unsigned,
            "private_contacts.self_sync",
        );
    }

    pub(super) fn send_private_contact_document(&mut self, document: &PrivateContactDocument) {
        self.send_private_contact_control(
            serde_json::json!({"type":"private-contact-sync", "v":1, "document":document}),
        );
    }

    pub(in crate::core) fn private_contact_snapshot(&self) -> Vec<PrivateContactDocument> {
        let Some(state) = self.private_contacts.state.as_ref() else {
            return Vec::new();
        };
        private_contact_documents(state)
    }

    pub(in crate::core) fn receive_private_contact_control(
        &mut self,
        owner: PublicKey,
        device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        if content.len() > 32_768
            || self
                .logged_in
                .as_ref()
                .is_none_or(|login| login.owner_pubkey != owner)
            || !device.is_some_and(|device| self.device_sync_peer_is_authorized(&device.to_hex()))
        {
            return true;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
            return true;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("private-contact-sync")
            || value.get("v").and_then(serde_json::Value::as_u64) != Some(1)
        {
            return true;
        }
        if value.get("request").is_some() {
            if value.get("request").and_then(serde_json::Value::as_bool) != Some(true)
                || value.get("document").is_some()
            {
                return true;
            }
            let device = device.map(|device| device.to_hex()).unwrap_or_default();
            let now = unix_now().get();
            if self
                .private_contacts
                .last_requests
                .get(&device)
                .is_some_and(|last| now.saturating_sub(*last) < 30)
            {
                return true;
            }
            if self.private_contact_state().is_err() {
                return false;
            }
            self.private_contacts.last_requests.insert(device, now);
            for document in self.private_contact_snapshot() {
                self.send_private_contact_document(&document);
            }
        } else if let Some(raw) = value.get("document") {
            let Ok(document) = serde_json::from_value::<PrivateContactDocument>(raw.clone()) else {
                return true;
            };
            if self.merge_private_contact_from_sibling(&document) {
                self.rebuild_persist_and_emit_state();
            }
        }
        true
    }
}
