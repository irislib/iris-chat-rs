impl ProtocolEngine {
    pub fn ingest_app_keys_snapshot(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        self.ingest_app_keys_snapshot_with_raw_event_json(owner_pubkey, app_keys, created_at, None)
    }

    pub fn ingest_app_keys_snapshot_with_raw_event_json(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
        raw_event_json: Option<String>,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let checkpoint = self.state_checkpoint();
        let owner = ndr_owner(owner_pubkey);
        let roster = DeviceRoster::new(
            NdrUnixSeconds(created_at),
            app_keys
                .get_all_devices()
                .into_iter()
                .map(|entry| {
                    AuthorizedDevice::new(
                        ndr_device(entry.identity_pubkey),
                        NdrUnixSeconds(entry.created_at),
                    )
                })
                .collect(),
        );
        let decision = if owner_pubkey == self.owner_pubkey {
            if should_replace_provisional_local_roster(
                &self.session_manager.snapshot(),
                self.owner_pubkey,
                self.local_device,
                &roster,
            ) {
                self.session_manager.replace_local_roster(roster)
            } else {
                self.session_manager.apply_local_roster(roster)
            }
        } else {
            self.session_manager.observe_peer_roster(owner, roster)
        };
        if owner_pubkey == self.owner_pubkey {
            if let Some(raw_event_json) = raw_event_json {
                match app_keys_owner_roster_proof(&raw_event_json, self.local_device) {
                    Ok(proof) => {
                        if self.session_manager.set_local_owner_roster_proof(proof).is_err() {
                            self.session_manager.clear_local_owner_roster_proof();
                        }
                    }
                    Err(_) => self.session_manager.clear_local_owner_roster_proof(),
                }
            }
        }
        if matches!(
            decision,
            nostr_double_ratchet::RosterSnapshotDecision::Stale
        ) {
            return Ok(ProtocolRetryBatch::default());
        }
        self.invalidate_known_message_author_cache();
        if let Err(error) = self.persist() {
            self.restore_checkpoint(checkpoint);
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        if owner_pubkey == self.owner_pubkey {
            self.local_app_keys_observed = true;
        }
        Ok(ProtocolRetryBatch::default())
    }

    pub fn observe_invite_event(&mut self, event: &Event) -> anyhow::Result<ProtocolRetryBatch> {
        let session_checkpoint = self.session_manager.clone();
        let mut invite = parse_invite_event(event)?;
        let invite_owner = invite
            .inviter_owner_pubkey
            .or_else(|| self.verified_roster_owner_for_device(invite.inviter_device_pubkey))
            .unwrap_or_else(|| NdrOwnerPubkey::from_bytes(invite.inviter_device_pubkey.to_bytes()));
        if invite.inviter_owner_pubkey.is_none() {
            invite.inviter_owner_pubkey = Some(invite_owner);
        }
        if invite.inviter_device_pubkey != self.local_device {
            self.session_manager
                .observe_device_invite(invite_owner, invite)?;
            self.invalidate_known_message_author_cache();
        }
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        Ok(ProtocolRetryBatch::default())
    }

    pub fn observe_invite_response_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let Some(local_invite_recipient) = self
            .session_manager
            .snapshot()
            .local_invite
            .as_ref()
            .map(|invite| invite.inviter_ephemeral_public_key)
        else {
            return Ok(ProtocolRetryBatch::default());
        };
        let envelope = parse_invite_response_event(event)?;
        if envelope.recipient != local_invite_recipient {
            return Ok(ProtocolRetryBatch::default());
        }
        let session_checkpoint = self.session_manager.clone();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let _processed = match self.session_manager.observe_invite_response_with_roster_proof_verifier(
            &mut ctx,
            &envelope,
            |raw_proof, responder_device| {
                app_keys_owner_roster_proof(raw_proof, responder_device).map_err(|error| {
                    DomainError::InvalidState(format!("invalid owner roster proof: {error}")).into()
                })
            },
        ) {
            Ok(processed) => processed,
            Err(NdrError::Domain(DomainError::InviteAlreadyUsed)) => {
                return Ok(ProtocolRetryBatch::default());
            }
            Err(error) => return Err(error.into()),
        };
        self.invalidate_known_message_author_cache();
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        Ok(ProtocolRetryBatch::default())
    }

    pub fn accept_invite(
        &mut self,
        invite: &Invite,
        owner_pubkey_hint: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolAcceptInviteResult> {
        let invite_owner = if let Some(owner) = owner_pubkey_hint.or_else(|| {
            invite
                .inviter_owner_pubkey
                .and_then(|owner| public_owner(owner).ok())
        }) {
            owner
        } else {
            public_device(invite.inviter_device_pubkey)?
        };
        let mut invite = invite.clone();
        invite.inviter_owner_pubkey = Some(ndr_owner(invite_owner));
        self.session_manager
            .observe_device_invite(ndr_owner(invite_owner), invite.clone())?;
        self.session_manager.observe_peer_roster(
            ndr_owner(invite_owner),
            DeviceRoster::new(
                NdrUnixSeconds(unix_now().get()),
                vec![AuthorizedDevice::new(
                    invite.inviter_device_pubkey,
                    invite.created_at,
                )],
            ),
        );
        self.invalidate_known_message_author_cache();
        // Bootstrap the session by sending a typing rumor with an
        // already-elapsed expiration. We need the inner kind-1060 publish to
        // make the inviter create their side of the session (otherwise the
        // inviter never learns our session ephemeral pubkey and their replies
        // never reach this device, matching what
        // `SessionManager.acceptInvite` does in TypeScript iris-chat).
        // The expired expiration is the same shape as `stop_typing`, so the
        // receiver treats this rumor as "stop typing" and does not flash a
        // typing indicator for a chat the user hasn't started typing in.
        let now = unix_now();
        let typing = pairwise_codec::typing_event(
            self.owner_pubkey,
            pairwise_codec::EncodeOptions::new(now.get(), current_unix_millis()).with_expiration(1),
        )?;
        let mut bootstrap =
            self.send_direct_unsigned_event(invite_owner, &invite_owner.to_hex(), typing, now)?;
        for effect in &mut bootstrap.effects {
            let ProtocolEffect::Publish(publish) = effect;
            publish.inner_event_id = None;
        }
        Ok(ProtocolAcceptInviteResult {
            owner_pubkey: invite_owner,
            inviter_device_pubkey: public_device(invite.inviter_device_pubkey)?,
            device_id: public_device(invite.inviter_device_pubkey)?.to_hex(),
            effects: bootstrap.effects,
        })
    }

    pub fn import_session_state(
        &mut self,
        peer_pubkey: PublicKey,
        device_id: Option<String>,
        state: SessionState,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let device_pubkey = device_id
            .as_deref()
            .and_then(|value| PublicKey::parse(value).ok())
            .map(ndr_device)
            .unwrap_or_else(|| ndr_device(peer_pubkey));
        self.session_manager.import_session_state(
            ndr_owner(peer_pubkey),
            device_pubkey,
            state,
            NdrUnixSeconds(now.get()),
        );
        self.invalidate_known_message_author_cache();
        self.persist()?;
        Ok(ProtocolRetryBatch::default())
    }

    pub fn create_group(
        &mut self,
        name: String,
        member_owners: Vec<PublicKey>,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
            let result = engine.group_manager.create_group_with_protocol(
                &mut engine.session_manager,
                &mut ctx,
                name,
                member_owners.into_iter().map(ndr_owner).collect(),
                GroupProtocol::sender_key_v1(),
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&result.prepared, None)?;
            output.snapshot = Some(result.group);
            engine.persist()?;
            Ok(output)
        })
    }

    fn ensure_supported_group_protocol(&self, group_id: &str) -> anyhow::Result<()> {
        if self
            .group_manager
            .group(group_id)
            .is_some_and(|group| !group.protocol.is_sender_key_v1())
        {
            anyhow::bail!("group `{group_id}` uses an unsupported legacy group protocol");
        }
        Ok(())
    }

    pub fn update_group_name(
        &mut self,
        group_id: &str,
        name: String,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.update_name(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                name,
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn update_group_picture(
        &mut self,
        group_id: &str,
        picture: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.update_picture(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                picture,
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn update_group_about(
        &mut self,
        group_id: &str,
        about: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.update_about(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                about,
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn add_group_members(
        &mut self,
        group_id: &str,
        members: Vec<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.add_members(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                members.into_iter().map(ndr_owner).collect(),
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn remove_group_member(
        &mut self,
        group_id: &str,
        member: PublicKey,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.remove_members(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                vec![ndr_owner(member)],
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn set_group_admin(
        &mut self,
        group_id: &str,
        member: PublicKey,
        is_admin: bool,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = if is_admin {
                engine.group_manager.add_admins(
                    &mut engine.session_manager,
                    &mut ctx,
                    group_id,
                    vec![ndr_owner(member)],
                )?
            } else {
                engine.group_manager.remove_admins(
                    &mut engine.session_manager,
                    &mut ctx,
                    group_id,
                    vec![ndr_owner(member)],
                )?
            };
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn send_group_payload(
        &mut self,
        group_id: &str,
        payload: Vec<u8>,
        inner_event_id: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.send_message(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                payload,
            )?;
            let message_id = inner_event_id.clone();
            let mut output = engine.protocol_group_send_from_prepared(&prepared, inner_event_id)?;
            output.snapshot = engine.group_manager.group(group_id);
            output.message_id = message_id;
            engine.persist()?;
            Ok(output)
        })
    }

    pub fn send_direct_text(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        text: &str,
        expires_at_secs: Option<u64>,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let now_ms = current_unix_millis();
        let mut options = pairwise_codec::EncodeOptions::new(now.get(), now_ms);
        if let Some(expires_at_secs) = expires_at_secs {
            options = options.with_expiration(expires_at_secs);
        }
        let rumor = pairwise_codec::message_event(self.owner_pubkey, text.to_string(), options)?;
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let remote_payload = serde_json::to_vec(&rumor)?;
        self.send_direct_payloads(
            peer_pubkey,
            chat_id,
            remote_payload.clone(),
            local_sibling_payload(peer_pubkey, &remote_payload)?,
            Some(message_id.clone()),
            message_id,
            now,
        )
    }

    pub fn send_direct_unsigned_event(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        mut rumor: UnsignedEvent,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        rumor.ensure_id();
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let remote_payload = serde_json::to_vec(&rumor)?;
        self.send_direct_payloads(
            peer_pubkey,
            chat_id,
            remote_payload.clone(),
            local_sibling_payload(peer_pubkey, &remote_payload)?,
            Some(message_id.clone()),
            message_id,
            now,
        )
    }

    pub fn send_direct_unsigned_event_to_peer_only(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        mut rumor: UnsignedEvent,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        rumor.ensure_id();
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let remote_payload = serde_json::to_vec(&rumor)?;
        self.send_direct_remote_payload(
            peer_pubkey,
            chat_id,
            remote_payload,
            Some(message_id.clone()),
            message_id,
            now,
        )
    }

    pub fn send_local_sibling_unsigned_event(
        &mut self,
        conversation_owner: PublicKey,
        chat_id: &str,
        mut rumor: UnsignedEvent,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        rumor.ensure_id();
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let payload = serde_json::to_vec(&rumor)?;
        self.send_local_sibling_payload(
            chat_id,
            local_sibling_payload(conversation_owner, &payload)?,
            Some(message_id.clone()),
            message_id,
            now,
        )
    }

    fn send_direct_remote_payload(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        remote_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.send_direct_remote_payload_inner(
                peer_pubkey,
                chat_id,
                remote_payload,
                inner_event_id,
                message_id,
                now,
            )
        })
    }

    fn send_local_sibling_payload(
        &mut self,
        chat_id: &str,
        local_sibling_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.send_local_sibling_payload_inner(
                chat_id,
                local_sibling_payload,
                inner_event_id,
                message_id,
                now,
            )
        })
    }

    fn send_direct_payloads(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        remote_payload: Vec<u8>,
        local_sibling_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        self.with_state_checkpoint(|engine| {
            engine.send_direct_payloads_inner(
                peer_pubkey,
                chat_id,
                remote_payload,
                local_sibling_payload,
                inner_event_id,
                message_id,
                now,
            )
        })
    }

    fn send_direct_remote_payload_inner(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        remote_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let recipient_owner = ndr_owner(peer_pubkey);
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
        let remote = self.session_manager.prepare_remote_send(
            &mut ctx,
            recipient_owner,
            remote_payload.clone(),
        )?;

        let mut event_ids = Vec::new();
        let effects = protocol_effects_from_prepared(
            &remote,
            inner_event_id.clone(),
            chat_id.to_string(),
            &mut event_ids,
        )?;

        let _ = (recipient_owner, remote_payload, inner_event_id);
        self.persist()?;
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
        })
    }

    fn send_local_sibling_payload_inner(
        &mut self,
        chat_id: &str,
        local_sibling_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
        let local = self
            .session_manager
            .prepare_local_sibling_send_reusing_sessions(&mut ctx, local_sibling_payload.clone())?;

        let mut event_ids = Vec::new();
        let effects = protocol_effects_from_prepared(
            &local,
            inner_event_id.clone(),
            chat_id.to_string(),
            &mut event_ids,
        )?;

        let _ = (local_sibling_payload, inner_event_id);
        self.persist()?;
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
        })
    }

    fn send_direct_payloads_inner(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        remote_payload: Vec<u8>,
        local_sibling_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let recipient_owner = ndr_owner(peer_pubkey);
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
        let remote = self.session_manager.prepare_remote_send(
            &mut ctx,
            recipient_owner,
            remote_payload.clone(),
        )?;
        let local = self
            .session_manager
            .prepare_local_sibling_send_reusing_sessions(&mut ctx, local_sibling_payload.clone())?;

        let mut event_ids = Vec::new();
        let mut effects = Vec::new();
        effects.extend(protocol_effects_from_prepared(
            &remote,
            inner_event_id.clone(),
            chat_id.to_string(),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_prepared(
            &local,
            inner_event_id.clone(),
            chat_id.to_string(),
            &mut event_ids,
        )?);

        let _ = (remote_payload, local_sibling_payload, inner_event_id);
        self.persist()?;
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
        })
    }
}

fn app_keys_owner_roster_proof(
    raw_event_json: &str,
    expected_device: NdrDevicePubkey,
) -> anyhow::Result<OwnerRosterProof> {
    let event: Event = serde_json::from_str(raw_event_json)?;
    let app_keys = AppKeys::from_event(&event)?;
    let roster = DeviceRoster::new(
        NdrUnixSeconds(event.created_at.as_secs()),
        app_keys
            .get_all_devices()
            .into_iter()
            .map(|entry| {
                AuthorizedDevice::new(
                    ndr_device(entry.identity_pubkey),
                    NdrUnixSeconds(entry.created_at),
                )
            })
            .collect(),
    );
    let proof = OwnerRosterProof::new(ndr_owner(event.pubkey), roster, raw_event_json.to_string());
    proof.ensure_authorizes_device(expected_device)?;
    Ok(proof)
}
