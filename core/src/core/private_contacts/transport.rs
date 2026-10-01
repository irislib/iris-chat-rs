use super::*;

impl AppCore {
    pub(in crate::core) fn receive_private_runtime_control(
        &mut self,
        owner: PublicKey,
        device: Option<PublicKey>,
        rumor: &RuntimeRumor,
    ) -> Option<bool> {
        if let Some(disposition) = self.private_sibling_control_disposition(owner, device, rumor) {
            return Some(disposition);
        }
        match rumor.kind {
            // An old client would bridge these facts into static-key relay records.
            10451 => Some(true),
            PRIVATE_CONTACT_CONTROL_KIND => {
                Some(self.receive_private_contact_control(owner, device, &rumor.content))
            }
            private_device_labels::DEVICE_LABEL_CONTROL_KIND => {
                Some(self.receive_private_device_label_control(owner, device, &rumor.content))
            }
            calls::push::CALL_OFFER_KIND => Some(self.receive_ratcheted_call_offer(
                owner,
                device,
                &rumor.content,
                rumor.created_at_secs,
            )),
            _ => None,
        }
    }

    pub(in crate::core) fn start_private_contact_sync(&mut self) {
        if let Err(error) = self.private_contact_state() {
            self.push_debug_log("private_contacts.restore_failed", error.to_string());
            return;
        }
        self.recover_private_contact_history();
        self.kick_private_contact_sync();
    }

    // A fresh device asks authorized siblings for current registers. No owner-key
    // encrypted relay history is read, including after upgrades.
    pub(in crate::core) fn recover_private_contact_history(&mut self) {
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return;
        };
        if let Ok(control) = build_private_contact_request_v2(&owner) {
            self.send_private_contact_control(&control);
        }
        self.broadcast_private_device_labels();
    }

    pub(in crate::core) fn private_contact_sync_tick(&mut self, generation: u64) {
        if generation != self.private_contacts.generation
            || self.logged_in.is_none()
            || self.suspended
        {
            return;
        }
        self.flush_private_contact_events();
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_secs(30)).await;
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
        let documents = self
            .private_contacts
            .state
            .as_ref()
            .map(pending_private_contacts_v2)
            .unwrap_or_default();
        for document in documents {
            if !self.send_private_contact_document(&document) {
                continue;
            }
            let result = (|| -> anyhow::Result<()> {
                let state = self.private_contact_state()?;
                let next = acknowledge_private_contact_document_v2(&state, &document)?;
                self.commit_private_contacts(next)
            })();
            if let Err(error) = result {
                self.push_debug_log("private_contacts.ack_failed", error.to_string());
            }
        }
    }
    // Consume obsolete private relay records without decrypting or bridging them.
    pub(in crate::core) fn receive_private_contact_event(&mut self, event: &Event) -> bool {
        obsolete_private_contact_event(event)
    }
    fn send_private_contact_control(&mut self, value: &PrivateContactControlV2) -> bool {
        let Some(owner) = self.logged_in.as_ref().map(|login| login.owner_pubkey) else {
            return false;
        };
        let Ok(content) = serde_json::to_string(value) else {
            return false;
        };
        let unsigned = EventBuilder::new(Kind::from(PRIVATE_CONTACT_CONTROL_KIND as u16), content)
            .tag(nostr::Tag::public_key(owner))
            .build(owner);
        self.send_protocol_engine_unsigned_event_to_local_siblings(
            owner,
            &owner.to_hex(),
            unsigned,
            "private_contacts.self_sync_v2",
        )
    }
    pub(super) fn send_private_contact_document(
        &mut self,
        document: &PrivateContactDocumentV2,
    ) -> bool {
        build_private_contact_control_v2(document)
            .is_ok_and(|control| self.send_private_contact_control(&control))
    }
    pub(in crate::core) fn private_contact_snapshot(&self) -> Vec<PrivateContactDocumentV2> {
        self.private_contacts
            .state
            .as_ref()
            .map(private_contact_documents_v2)
            .unwrap_or_default()
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
        let Ok(control) = parse_private_contact_control_v2(&value, &owner.to_hex()) else {
            return true;
        };
        let result = (|| -> anyhow::Result<()> {
            let state = self.private_contact_state()?;
            match control {
                PrivateContactControlV2::Request { .. } => {
                    let device = device.map(|key| key.to_hex()).unwrap_or_default();
                    let now = unix_now().get();
                    if self
                        .private_contacts
                        .last_requests
                        .get(&device)
                        .is_some_and(|last| now.saturating_sub(*last) < 30)
                    {
                        return Ok(());
                    }
                    self.commit_private_contacts(queue_private_contact_snapshot_v2(&state))?;
                    self.private_contacts.last_requests.insert(device, now);
                    self.replay_private_device_labels();
                    self.replay_private_chat_settings();
                    self.flush_private_contact_events();
                }
                PrivateContactControlV2::Sync { document, .. } => {
                    let next = merge_private_contact_document_v2(&state, &document)?;
                    if next != state {
                        self.commit_private_contacts(next)?;
                        self.rebuild_persist_and_emit_state();
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.push_debug_log("private_contacts.receive_failed", error.to_string());
            return false;
        }
        true
    }
}

pub(in crate::core) fn obsolete_private_contact_event(event: &Event) -> bool {
    event.kind.as_u16() == 30078
        && event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", "nostr-social-memory/v1"])
}
