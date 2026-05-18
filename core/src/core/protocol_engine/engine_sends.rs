impl ProtocolEngine {
    pub(super) fn ingest_app_keys_snapshot(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let session_checkpoint = self.session_manager.clone();
        let latest_checkpoint = self.latest_app_keys_created_at.clone();
        let pending_outbound_checkpoint = self.pending_outbound.clone();
        let pending_inbound_checkpoint = self.pending_inbound.clone();
        let pending_group_fanouts_checkpoint = self.pending_group_fanouts.clone();
        let pending_group_pairwise_checkpoint = self.pending_group_pairwise_payloads.clone();
        let owner_hex = owner_pubkey.to_hex();
        let latest = self
            .latest_app_keys_created_at
            .get(&owner_hex)
            .copied()
            .unwrap_or(0);
        if created_at < latest {
            return Ok(ProtocolRetryBatch::default());
        }
        self.latest_app_keys_created_at
            .insert(owner_hex, created_at);
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
        if owner_pubkey == self.owner_pubkey {
            self.session_manager.replace_local_roster(roster);
        } else {
            self.session_manager
                .observe_peer_roster(ndr_owner(owner_pubkey), roster);
        }
        self.invalidate_known_message_author_cache();
        self.wake_pending_protocol_for_owner(ndr_owner(owner_pubkey));
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.latest_app_keys_created_at = latest_checkpoint;
            self.pending_outbound = pending_outbound_checkpoint;
            self.pending_inbound = pending_inbound_checkpoint;
            self.pending_group_fanouts = pending_group_fanouts_checkpoint;
            self.pending_group_pairwise_payloads = pending_group_pairwise_checkpoint;
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        self.retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
    }

    pub(super) fn observe_invite_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let session_checkpoint = self.session_manager.clone();
        let pending_outbound_checkpoint = self.pending_outbound.clone();
        let pending_inbound_checkpoint = self.pending_inbound.clone();
        let pending_group_fanouts_checkpoint = self.pending_group_fanouts.clone();
        let pending_group_pairwise_checkpoint = self.pending_group_pairwise_payloads.clone();
        let invite = parse_invite_event(event)?;
        let invite_owner = invite
            .inviter_owner_pubkey
            .unwrap_or_else(|| NdrOwnerPubkey::from_bytes(invite.inviter_device_pubkey.to_bytes()));
        if invite.inviter_device_pubkey != self.local_device {
            self.session_manager
                .observe_device_invite(invite_owner, invite)?;
            self.invalidate_known_message_author_cache();
            self.wake_pending_protocol_for_owner(invite_owner);
        }
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.pending_outbound = pending_outbound_checkpoint;
            self.pending_inbound = pending_inbound_checkpoint;
            self.pending_group_fanouts = pending_group_fanouts_checkpoint;
            self.pending_group_pairwise_payloads = pending_group_pairwise_checkpoint;
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        self.retry_pending_protocol(NdrUnixSeconds(event.created_at.as_secs()))
    }

    pub(super) fn observe_invite_response_event(
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
        let pending_outbound_checkpoint = self.pending_outbound.clone();
        let pending_inbound_checkpoint = self.pending_inbound.clone();
        let pending_group_fanouts_checkpoint = self.pending_group_fanouts.clone();
        let pending_group_pairwise_checkpoint = self.pending_group_pairwise_payloads.clone();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let processed = self
            .session_manager
            .observe_invite_response(&mut ctx, &envelope)?;
        self.invalidate_known_message_author_cache();
        if let Some(processed) = processed.as_ref() {
            self.wake_pending_protocol_for_owner(
                processed
                    .claimed_owner_pubkey
                    .unwrap_or(processed.owner_pubkey),
            );
        }
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.pending_outbound = pending_outbound_checkpoint;
            self.pending_inbound = pending_inbound_checkpoint;
            self.pending_group_fanouts = pending_group_fanouts_checkpoint;
            self.pending_group_pairwise_payloads = pending_group_pairwise_checkpoint;
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        self.retry_pending_protocol(ctx.now)
    }

    pub(super) fn accept_invite(
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
        let bootstrap =
            self.send_direct_unsigned_event(invite_owner, &invite_owner.to_hex(), typing, now)?;
        Ok(ProtocolAcceptInviteResult {
            owner_pubkey: invite_owner,
            inviter_device_pubkey: public_device(invite.inviter_device_pubkey)?,
            device_id: public_device(invite.inviter_device_pubkey)?.to_hex(),
            effects: bootstrap.effects,
        })
    }

    pub(super) fn import_session_state(
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
        self.retry_pending_protocol(NdrUnixSeconds(now.get()))
    }

    pub(super) fn create_group(
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

    pub(super) fn update_group_name(
        &mut self,
        group_id: &str,
        name: String,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
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

    pub(super) fn add_group_members(
        &mut self,
        group_id: &str,
        members: Vec<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
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

    pub(super) fn remove_group_member(
        &mut self,
        group_id: &str,
        member: PublicKey,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
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

    pub(super) fn set_group_admin(
        &mut self,
        group_id: &str,
        member: PublicKey,
        is_admin: bool,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
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

    pub(super) fn send_group_payload(
        &mut self,
        group_id: &str,
        payload: Vec<u8>,
        inner_event_id: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.with_state_checkpoint(|engine| {
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

    pub(super) fn send_direct_text(
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

    pub(super) fn send_direct_unsigned_event(
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
            .prepare_local_sibling_send_refreshing_one_way_sessions(
                &mut ctx,
                local_sibling_payload.clone(),
            )?;

        let mut event_ids = Vec::new();
        let mut effects = Vec::new();
        effects.extend(protocol_effects_from_prepared(
            &remote,
            inner_event_id.clone(),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_prepared(
            &local,
            inner_event_id.clone(),
            &mut event_ids,
        )?);

        let remote_delivered = delivered_device_hexes(&remote);
        let local_delivered = delivered_device_hexes(&local);
        let probe_local_sibling_roster = self.needs_local_sibling_roster_probe(&local);
        let has_undelivered_local_siblings = !self
            .remaining_local_sibling_targets(&local_delivered)
            .is_empty();
        let gaps = remote
            .relay_gaps
            .iter()
            .chain(local.relay_gaps.iter())
            .cloned()
            .collect::<Vec<_>>();
        let mut queued_targets = Vec::new();
        let mut queued_effects = Vec::new();
        if !gaps.is_empty() || probe_local_sibling_roster || has_undelivered_local_siblings {
            let pending = ProtocolPendingOutbound {
                message_id: message_id.clone(),
                chat_id: chat_id.to_string(),
                recipient_owner_hex: peer_pubkey.to_hex(),
                remote_payload,
                local_sibling_payload: Some(local_sibling_payload),
                inner_event_id,
                delivered_remote_device_hexes: remote_delivered,
                delivered_local_device_hexes: local_delivered,
                probe_local_sibling_roster,
                created_at_secs: now.get(),
                next_retry_at_secs: now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                reason: pending_reason_from_gaps(&gaps),
            };
            queued_targets = self.pending_target_hexes(&pending);
            queued_effects = self.protocol_backfill_effects_for_pending_outbound(
                &pending,
                NdrUnixSeconds(now.get()),
                "direct_send",
            );
            self.upsert_pending_outbound(pending);
        }
        self.persist()?;
        effects.extend(queued_effects);
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
            queued_targets,
        })
    }

}
