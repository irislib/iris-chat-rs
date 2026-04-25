use super::*;

impl AppCore {
    pub(super) fn start_pending_message_publish(
        &mut self,
        message_id: String,
        chat_id: String,
        message_events: Vec<Event>,
    ) {
        self.start_ordinary_publish(message_id, chat_id, message_events);
    }

    pub(super) fn start_publish_for_pending(
        &mut self,
        message_id: String,
        chat_id: String,
        publish_mode: OutboundPublishMode,
        batch: PreparedPublishBatch,
    ) {
        self.request_protocol_subscription_refresh();
        if batch.message_events.is_empty() {
            return;
        }

        match publish_mode {
            OutboundPublishMode::OrdinaryFirstAck => {
                self.start_pending_message_publish(message_id, chat_id, batch.message_events);
            }
            OutboundPublishMode::FirstContactStaged => {
                self.start_staged_first_contact_send(StagedOutboundSend {
                    message_id,
                    chat_id,
                    invite_events: batch.invite_events,
                    message_events: batch.message_events,
                });
            }
            OutboundPublishMode::WaitForPeer => {}
        }
    }

    pub(super) fn reconcile_recent_handshake_peers(&mut self) -> Vec<(String, String)> {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return Vec::new();
        };

        let mut session_owners_by_device = BTreeMap::new();
        for user in logged_in.session_manager.snapshot().users {
            let owner_hex = user.owner_pubkey.to_string();
            for device in user.devices {
                if device.active_session.is_none()
                    && device.inactive_sessions.is_empty()
                    && device.claimed_owner_pubkey.is_none()
                {
                    continue;
                }
                session_owners_by_device
                    .insert(device.device_pubkey.to_string(), owner_hex.clone());
            }
        }

        let mut migrated_owner_hexes = Vec::new();
        for peer in self.recent_handshake_peers.values_mut() {
            let Some(owner_hex) = session_owners_by_device.get(&peer.device_hex) else {
                continue;
            };
            if *owner_hex != peer.owner_hex {
                let previous_owner_hex = peer.owner_hex.clone();
                peer.owner_hex = owner_hex.clone();
                migrated_owner_hexes.push((previous_owner_hex, owner_hex.clone()));
            }
        }

        migrated_owner_hexes
    }

    pub(super) fn apply_owner_migrations(&mut self, migrations: &[(String, String)]) {
        for (old_owner_hex, new_owner_hex) in migrations {
            if old_owner_hex == new_owner_hex {
                continue;
            }

            if let Some(mut old_thread) = self.threads.remove(old_owner_hex) {
                old_thread.chat_id = new_owner_hex.clone();
                for message in &mut old_thread.messages {
                    message.chat_id = new_owner_hex.clone();
                }

                match self.threads.get_mut(new_owner_hex) {
                    Some(existing) => {
                        existing.unread_count = existing
                            .unread_count
                            .saturating_add(old_thread.unread_count);
                        existing.updated_at_secs =
                            existing.updated_at_secs.max(old_thread.updated_at_secs);
                        existing.messages.extend(old_thread.messages);
                        existing.messages.sort_by(|left, right| {
                            left.created_at_secs
                                .cmp(&right.created_at_secs)
                                .then_with(|| left.id.cmp(&right.id))
                        });
                    }
                    None => {
                        self.threads.insert(new_owner_hex.clone(), old_thread);
                    }
                }
            }

            if self.active_chat_id.as_deref() == Some(old_owner_hex.as_str()) {
                self.active_chat_id = Some(new_owner_hex.clone());
            }
            for pending in &mut self.pending_outbound {
                if pending.chat_id == *old_owner_hex {
                    pending.chat_id = new_owner_hex.clone();
                }
            }
            for screen in &mut self.screen_stack {
                if let Screen::Chat { chat_id } = screen {
                    if *chat_id == *old_owner_hex {
                        *chat_id = new_owner_hex.clone();
                    }
                }
            }
        }
    }

    pub(super) fn start_staged_first_contact_send(&mut self, staged: StagedOutboundSend) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };

        for event in staged
            .invite_events
            .iter()
            .chain(staged.message_events.iter())
        {
            self.remember_event(event.id.to_string());
        }

        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            let invite_publish = publish_events_with_retry(
                &client,
                &relay_urls,
                staged.invite_events,
                "invite response",
            )
            .await;
            if invite_publish.is_err() {
                let _ = tx.send(CoreMsg::Internal(Box::new(
                    InternalEvent::PublishFinished {
                        message_id: staged.message_id,
                        chat_id: staged.chat_id,
                        success: false,
                    },
                )));
                return;
            }

            sleep(Duration::from_millis(FIRST_CONTACT_STAGE_DELAY_MS)).await;

            let success =
                publish_events_with_retry(&client, &relay_urls, staged.message_events, "message")
                    .await
                    .is_ok();
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::PublishFinished {
                    message_id: staged.message_id,
                    chat_id: staged.chat_id,
                    success,
                },
            )));
        });
    }

    pub(super) fn start_ordinary_publish(
        &mut self,
        message_id: String,
        chat_id: String,
        events: Vec<Event>,
    ) {
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return;
        };

        for event in &events {
            self.remember_event(event.id.to_string());
        }

        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            let success = publish_events_first_ack(&client, &relay_urls, &events, "message")
                .await
                .is_ok();
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::PublishFinished {
                    message_id,
                    chat_id,
                    success,
                },
            )));
        });
    }

    pub(super) fn publish_local_identity_artifacts(&self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        if logged_in.authorization_state == LocalAuthorizationState::Revoked {
            return;
        }

        let snapshot = logged_in.session_manager.snapshot();
        let local_roster = snapshot
            .users
            .iter()
            .find(|user| user.owner_pubkey == logged_in.owner_pubkey)
            .and_then(|user| user.roster.clone());
        let local_invite = snapshot.local_invite.clone();
        let owner_keys = logged_in.owner_keys.clone();
        let device_keys = logged_in.device_keys.clone();
        let owner_pubkey = logged_in.owner_pubkey;
        let local_profile = self.owner_profiles.get(&owner_pubkey.to_string()).cloned();
        let client = logged_in.client.clone();
        let relay_urls = logged_in.relay_urls.clone();
        let tx = self.core_sender.clone();

        self.runtime.spawn(async move {
            if let (Some(keys), Some(profile)) = (owner_keys.clone(), local_profile) {
                if profile.preferred_label().is_some() {
                    let event =
                        EventBuilder::new(Kind::Metadata, build_profile_metadata_json(&profile))
                            .sign_with_keys(&keys);
                    match event {
                        Ok(event) => {
                            let _ =
                                publish_event_with_retry(&client, &relay_urls, event, "metadata")
                                    .await;
                        }
                        Err(_) => {}
                    }
                }
            }

            if let (Some(keys), Some(roster)) = (owner_keys, local_roster) {
                let roster_event = match codec::roster_unsigned_event(owner_pubkey, &roster)
                    .and_then(|unsigned| unsigned.sign_with_keys(&keys).map_err(Into::into))
                {
                    Ok(event) => Some(event),
                    Err(_) => None,
                };
                if let Some(roster_event) = roster_event {
                    let _ = publish_event_with_retry(&client, &relay_urls, roster_event, "roster")
                        .await;
                }
            }

            if let Some(invite) = local_invite {
                let invite_event = match codec::invite_unsigned_event(&invite)
                    .and_then(|unsigned| unsigned.sign_with_keys(&device_keys).map_err(Into::into))
                {
                    Ok(event) => Some(event),
                    Err(_) => None,
                };
                if let Some(invite_event) = invite_event {
                    let _ = publish_event_with_retry(&client, &relay_urls, invite_event, "invite")
                        .await;
                }
            }

            let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::SyncComplete)));
        });
    }

    pub(super) fn publish_roster_update(&self, roster: DeviceRoster) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let Some(owner_keys) = logged_in.owner_keys.clone() else {
            return;
        };
        let owner_pubkey = logged_in.owner_pubkey;
        let client = logged_in.client.clone();
        let relay_urls = logged_in.relay_urls.clone();
        let tx = self.core_sender.clone();

        self.runtime.spawn(async move {
            match codec::roster_unsigned_event(owner_pubkey, &roster)
                .and_then(|unsigned| unsigned.sign_with_keys(&owner_keys).map_err(Into::into))
            {
                Ok(event) => {
                    let _ = publish_event_with_retry(&client, &relay_urls, event, "roster").await;
                }
                Err(_) => {}
            }

            let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::SyncComplete)));
        });
    }

    pub(super) fn republish_local_identity_artifacts(&self) {
        self.publish_local_identity_artifacts();
    }
}
