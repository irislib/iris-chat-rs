impl ProtocolEngine {
    pub fn ingest_app_keys_snapshot(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        Ok(self
            .ingest_app_keys_snapshot_inner(owner_pubkey, app_keys, created_at, None)?
            .retry_batch)
    }

    pub fn ingest_app_keys_event(&mut self, event: &Event) -> anyhow::Result<ProtocolRetryBatch> {
        Ok(self.ingest_app_keys_event_with_outcome(event)?.retry_batch)
    }

    pub fn ingest_app_keys_event_with_outcome(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolMetadataIngestionResult> {
        let app_keys = AppKeys::from_event(event)?;
        self.ingest_app_keys_snapshot_inner(
            event.pubkey,
            app_keys,
            event.created_at.as_secs(),
            Some(event),
        )
    }

    fn ingest_app_keys_snapshot_inner(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
        source_event: Option<&Event>,
    ) -> anyhow::Result<ProtocolMetadataIngestionResult> {
        let owner = ndr_owner(owner_pubkey);
        // Authorization replay commonly leaves the engine unchanged. Retain
        // only the fields this admission step can mutate; copying every queued
        // fanout before discovering a duplicate makes cold restore quadratic.
        let previous_session = self.session_manager.clone();
        let previous_evidence = self.invite_owner_app_keys_evidence.get(&owner).cloned();
        let previously_verified = self.verified_app_keys_owners.contains(&owner);
        let previously_observed = self.local_app_keys_observed;
        let now = unix_now();
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
        let mut roster_changed = if owner_pubkey == self.owner_pubkey {
            if should_replace_provisional_local_roster(
                &self.session_manager.snapshot(),
                self.owner_pubkey,
                self.local_device,
                &roster,
            ) {
                !matches!(
                    self.session_manager.replace_local_roster(roster),
                    nostr_double_ratchet::RosterSnapshotDecision::Stale
                )
            } else {
                !matches!(
                    self.session_manager.apply_local_roster(roster),
                    nostr_double_ratchet::RosterSnapshotDecision::Stale
                )
            }
        } else if let Some(event) = source_event {
            self.session_manager
                .observe_peer_app_keys_event(event.clone(), NdrUnixSeconds(now.get()))?
        } else {
            !matches!(
                self.session_manager.observe_peer_roster(owner, roster),
                nostr_double_ratchet::RosterSnapshotDecision::Stale
            )
        };
        if roster_changed && (owner_pubkey == self.owner_pubkey || source_event.is_none()) {
            // Equal-time roster merges can be exact replays. Compare all of
            // the manager state, not just membership, so reconciliation of
            // claimed devices and ratchets is still persisted when it changes.
            roster_changed = self.session_manager.snapshot() != previous_session.snapshot();
        }
        let exact_source_is_valid = source_event.is_some_and(|event| {
            app_keys_event_is_acceptable_for_owner(owner_pubkey, event, now.get())
        });
        let exact_evidence_changed = if exact_source_is_valid {
            let event = source_event.expect("validated AppKeys source event");
            self.update_invite_owner_app_keys_evidence(owner, event, &app_keys)
        } else {
            false
        };
        // Roster projections continue to drive established protocol routing.
        // Invite acceptance is stricter and consults only exact signed evidence.
        let provenance_changed = if owner_pubkey == self.owner_pubkey || exact_source_is_valid {
            self.verified_app_keys_owners.insert(owner)
        } else {
            false
        };
        let local_observation_changed =
            owner_pubkey == self.owner_pubkey && !self.local_app_keys_observed;
        if owner_pubkey == self.owner_pubkey {
            self.local_app_keys_observed = true;
        }
        if !roster_changed
            && !exact_evidence_changed
            && !provenance_changed
            && !local_observation_changed
        {
            return Ok(ProtocolMetadataIngestionResult::default());
        }
        // A real authorization change can wake retry deadlines. Take the full
        // rollback generation only now, before that work, and restore the
        // admission fields to their pre-event values inside it.
        let mut checkpoint = self.state_checkpoint();
        checkpoint.session_manager = previous_session;
        checkpoint.local_app_keys_observed = previously_observed;
        if !previously_verified {
            checkpoint.verified_app_keys_owners.remove(&owner);
        }
        match previous_evidence {
            Some(evidence) => { checkpoint.invite_owner_app_keys_evidence.insert(owner, evidence); }
            None => { checkpoint.invite_owner_app_keys_evidence.remove(&owner); }
        }
        self.invalidate_known_message_author_cache();
        self.wake_pending_protocol_for_owner(owner);
        if let Err(error) = self.persist() {
            self.restore_checkpoint(checkpoint);
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        Ok(ProtocolMetadataIngestionResult {
            changed: true,
            retry_batch: self.retry_pending_protocol(NdrUnixSeconds(now.get()))?,
        })
    }

    pub fn observe_invite_event(&mut self, event: &Event) -> anyhow::Result<ProtocolRetryBatch> {
        Ok(self.observe_invite_event_with_outcome(event)?.retry_batch)
    }

    pub fn observe_invite_event_with_outcome(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolMetadataIngestionResult> {
        let mut invite = parse_invite_event(event)?;
        let invite_owner = invite
            .inviter_owner_pubkey
            .or_else(|| self.verified_roster_owner_for_device(invite.inviter_device_pubkey))
            .unwrap_or_else(|| NdrOwnerPubkey::from_bytes(invite.inviter_device_pubkey.to_bytes()));
        if invite.inviter_owner_pubkey.is_none() {
            invite.inviter_owner_pubkey = Some(invite_owner);
        }
        if invite.inviter_device_pubkey == self.local_device {
            return Ok(ProtocolMetadataIngestionResult::default());
        }
        let session_checkpoint = self.session_manager.clone();
        self.session_manager
            .observe_device_invite(invite_owner, invite)?;
        // The manager admits stale/equal-time invites too. Compare its complete
        // state so replay cannot rewrite every pending payload or reset retry
        // deadlines, while a real invite replacement still wakes delivery.
        if self.session_manager.snapshot() == session_checkpoint.snapshot() {
            return Ok(ProtocolMetadataIngestionResult::default());
        }
        let pending_inbound_checkpoint = self.pending_inbound.clone();
        let pending_group_fanouts_checkpoint = self.pending_group_fanouts.clone();
        let pending_group_pairwise_checkpoint = self.pending_group_pairwise_payloads.clone();
        self.invalidate_known_message_author_cache();
        self.wake_pending_protocol_for_owner(invite_owner);
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.invalidate_pending_inbound_retry_deadline();
            self.pending_inbound = pending_inbound_checkpoint;
            self.invalidate_pending_group_fanout_retry_deadline();
            self.pending_group_fanouts = pending_group_fanouts_checkpoint;
            self.pending_group_pairwise_payloads = pending_group_pairwise_checkpoint;
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        Ok(ProtocolMetadataIngestionResult {
            changed: true,
            retry_batch: self.retry_pending_protocol(NdrUnixSeconds(event.created_at.as_secs()))?,
        })
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
        let checkpoint = self.state_checkpoint();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let processed = match self
            .session_manager
            .observe_invite_response(&mut ctx, &envelope)
        {
            Ok(processed) => processed,
            Err(NdrError::Domain(DomainError::InviteAlreadyUsed)) => {
                return Ok(ProtocolRetryBatch::default());
            }
            Err(error) => return Err(error.into()),
        };
        self.invalidate_known_message_author_cache();
        if let Some(processed) = processed.as_ref() {
            self.wake_pending_protocol_for_owner(
                processed
                    .claimed_owner_pubkey
                    .unwrap_or(processed.owner_pubkey),
            );
        }
        if let Some(processed) = processed.as_ref() {
            if let Some(invite) = self.local_invite() {
                let proof_batch = match self.ingest_invite_response_owner_proof(
                    &invite,
                    event,
                    public_owner(
                        processed
                            .claimed_owner_pubkey
                            .unwrap_or(processed.owner_pubkey),
                    )?,
                    public_device(processed.device_pubkey)?,
                ) {
                    Ok(batch) => batch,
                    Err(error) => {
                        self.restore_checkpoint(checkpoint);
                        self.invalidate_known_message_author_cache();
                        return Err(error);
                    }
                };
                if !proof_batch.is_empty() {
                    return Ok(proof_batch);
                }
            }
        }
        if let Err(error) = self.persist() {
            self.restore_checkpoint(checkpoint);
            self.invalidate_known_message_author_cache();
            return Err(error);
        }
        self.retry_pending_protocol(ctx.now)
    }

    pub fn accept_invite(
        &mut self,
        invite: &Invite,
        owner_pubkey_hint: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolAcceptInviteOutcome> {
        let invite_owner = resolve_invite_owner(invite, owner_pubkey_hint)?;
        let inviter_device = public_device(invite.inviter_device_pubkey)?;
        let owner = ndr_owner(invite_owner);

        if invite_owner != inviter_device {
            let Some(authorized) = self.invite_owner_app_keys_membership(owner, inviter_device)
            else {
                return Ok(ProtocolAcceptInviteOutcome::Blocked(
                    ProtocolAcceptInviteBlock::MissingOwnerRoster {
                        owner_pubkey: invite_owner,
                        device_pubkey: inviter_device,
                    },
                ));
            };
            if !authorized {
                return Ok(ProtocolAcceptInviteOutcome::Blocked(
                    ProtocolAcceptInviteBlock::UnauthorizedDevice {
                        owner_pubkey: invite_owner,
                        device_pubkey: inviter_device,
                    },
                ));
            }
        }

        let mut invite = invite.clone();
        invite.inviter_owner_pubkey = Some(owner);
        let checkpoint = self.state_checkpoint();
        self.session_manager
            .observe_device_invite(owner, invite.clone())?;
        if invite_owner == inviter_device {
            let has_roster = self
                .session_manager
                .snapshot()
                .users
                .into_iter()
                .find(|user| user.owner_pubkey == owner)
                .and_then(|user| user.roster)
                .is_some();
            if !has_roster {
                self.session_manager.observe_peer_roster(
                    owner,
                    DeviceRoster::new(
                        NdrUnixSeconds(unix_now().get()),
                        vec![AuthorizedDevice::new(
                            invite.inviter_device_pubkey,
                            invite.created_at,
                        )],
                    ),
                );
            }
        }
        self.invalidate_known_message_author_cache();
        let now = unix_now();
        let result = (|| -> anyhow::Result<ProtocolAcceptInviteResult> {
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
            let (mut session, response) = invite.accept_with_owner_context(
                &mut ctx,
                self.local_device,
                self.local_device_secret,
                Some(self.local_owner),
            )?;

            // Publish the actual invite response, then an expired typing rumor
            // through the newly-created session. The second event bootstraps
            // the inviter's receiving state without showing a typing indicator.
            let response_event = self.cached_invite_response_with_owner_proof(&response)?;
            let mut typing = pairwise_codec::typing_event(
                self.owner_pubkey,
                pairwise_codec::EncodeOptions::new(now.get(), current_unix_millis())
                    .with_expiration(1),
            )?;
            typing.ensure_id();
            let typing_payload = serde_json::to_vec(&typing)?;
            let plan = session.plan_send(&typing_payload, ctx.now)?;
            let mut envelope = session.apply_send(plan).envelope;
            envelope.recipient = Some(invite.inviter_device_pubkey);
            let typing_event = message_event_for_delivery(&Delivery {
                owner_pubkey: owner,
                device_pubkey: invite.inviter_device_pubkey,
                envelope,
            })?;

            self.session_manager.import_session_state(
                owner,
                invite.inviter_device_pubkey,
                session.state,
                ctx.now,
            );
            self.invalidate_known_message_author_cache();
            self.persist()?;

            let chat_id = invite_owner.to_hex();
            Ok(ProtocolAcceptInviteResult {
                owner_pubkey: invite_owner,
                inviter_device_pubkey: inviter_device,
                device_id: inviter_device.to_hex(),
                effects: vec![
                    ProtocolEffect::Publish(ProtocolPublish {
                        authored_at_secs: Some(now.get()),
                        event: response_event,
                        chat_id: chat_id.clone(),
                        inner_event_id: None,
                    }),
                    ProtocolEffect::Publish(ProtocolPublish {
                        authored_at_secs: Some(now.get()),
                        event: typing_event,
                        chat_id,
                        inner_event_id: None,
                    }),
                ],
            })
        })();

        match result {
            Ok(result) => Ok(ProtocolAcceptInviteOutcome::Accepted(result)),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
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
        self.wake_pending_protocol_for_owner(ndr_owner(peer_pubkey));
        self.persist()?;
        self.retry_pending_protocol(NdrUnixSeconds(now.get()))
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

    /// Admission and its distinct-owner receipt share one durable checkpoint.
    /// Force persistence even in a receive batch: never publish membership before
    /// the use limit is durable. Retry after a crash cannot admit an extra owner.
    pub fn admit_group_invite_member(
        &mut self,
        group_id: &str,
        token: &str,
        member: PublicKey,
        max_uses: Option<u32>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        anyhow::ensure!(
            token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid invite token"
        );
        anyhow::ensure!(
            max_uses.is_none_or(|n| (1..=10000).contains(&n)),
            "Invalid invite limit"
        );
        self.with_state_checkpoint(|engine| {
            engine.ensure_supported_group_protocol(group_id)?;
            let group = engine
                .group_manager
                .group(group_id)
                .ok_or_else(|| anyhow::anyhow!("Group unavailable"))?;
            anyhow::ensure!(
                group.admins.contains(&engine.local_owner)
                    && group.members.contains(&engine.local_owner),
                "Admin required"
            );
            let member_owner = ndr_owner(member);
            let key = format!("{group_id}:{token}");
            let used = engine.group_invite_uses.get(&key);
            anyhow::ensure!(
                !used.is_some_and(|owners| owners.contains(&member_owner)),
                "Invite already used"
            );
            anyhow::ensure!(
                max_uses.is_none_or(|limit| used.map_or(0, BTreeSet::len) < limit as usize),
                "Invite is full"
            );
            anyhow::ensure!(!group.members.contains(&member_owner), "Already a member");
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
            let prepared = engine.group_manager.add_members(
                &mut engine.session_manager,
                &mut ctx,
                group_id,
                vec![member_owner],
            )?;
            let mut output = engine.protocol_group_send_from_prepared(&prepared, None)?;
            output.snapshot = engine.group_manager.group(group_id);
            engine
                .group_invite_uses
                .entry(key)
                .or_default()
                .insert(member_owner);
            engine.persist_now()?;
            Ok(output)
        })
    }

    pub fn group_invite_use_count(&self, group_id: &str, token: &str) -> usize {
        self.group_invite_uses
            .get(&format!("{group_id}:{token}"))
            .map_or(0, BTreeSet::len)
    }

    pub fn group_snapshot(&self, group_id: &str) -> Option<GroupSnapshot> {
        self.group_manager.group(group_id)
    }

    pub fn group_invite_used_by(&self, group_id: &str, token: &str, member: PublicKey) -> bool {
        self.group_invite_uses
            .get(&format!("{group_id}:{token}"))
            .is_some_and(|owners| owners.contains(&ndr_owner(member)))
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
            engine.prepare_private_group_wakes(group_id, &mut output, message_id.clone())?;
            // The shared ciphertext is only a payload inside recipient-specific
            // ratchets. It must never create a public channel or publication.
            let private_ids = output
                .effects
                .iter()
                .filter_map(|effect| {
                    let ProtocolEffect::Publish(publish) = effect;
                    (publish.event.tags.public_keys().next().is_some())
                        .then_some(publish.event.id.to_hex())
                })
                .collect::<HashSet<_>>();
            output.effects.retain(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                private_ids.contains(&publish.event.id.to_hex())
            });
            output.event_ids.retain(|id| private_ids.contains(id));
            output.has_pending_deliveries = engine.pending_group_fanouts.iter().any(|pending| {
                pending.group_id == group_id && pending.inner_event_id == message_id
            });
            engine.retain_group_publications(&output.effects, None)?;
            output.message_id = message_id;
            // The ratchets and every ready publication must survive before the
            // caller may hand this batch to any network carrier.
            engine.persist_now()?;
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
        self.send_direct_text_created_at(peer_pubkey, chat_id, text, expires_at_secs, now, now)
    }

    /// Keep the authored time separate from the transport/retry clock.
    #[allow(clippy::too_many_arguments)]
    pub fn send_direct_text_created_at(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        text: &str,
        expires_at_secs: Option<u64>,
        created_at: UnixSeconds,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let now_ms = current_unix_millis();
        let mut options = pairwise_codec::EncodeOptions::new(created_at.get(), now_ms);
        if let Some(expires_at_secs) = expires_at_secs {
            options = options.with_expiration(expires_at_secs);
        }
        let rumor = pairwise_codec::message_event(self.owner_pubkey, text.to_string(), options)?;
        self.send_direct_unsigned_event(peer_pubkey, chat_id, rumor, now)
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
        let sibling_payload = local_sibling_payload(peer_pubkey, &remote_payload)?;
        self.with_state_checkpoint(|engine| {
            if peer_pubkey == engine.owner_pubkey {
                engine.send_local_sibling_payload_inner(
                    chat_id,
                    sibling_payload,
                    Some(message_id.clone()),
                    message_id,
                    now,
                )
            } else {
                engine.send_direct_payloads_inner(
                    peer_pubkey,
                    chat_id,
                    remote_payload,
                    sibling_payload,
                    Some(message_id.clone()),
                    message_id,
                    now,
                )
            }
        })
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
        self.with_state_checkpoint(|engine| {
            engine.send_direct_remote_payload_inner(
                peer_pubkey,
                chat_id,
                remote_payload,
                Some(message_id.clone()),
                message_id,
                now,
            )
        })
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
        let sibling_payload = local_sibling_payload(conversation_owner, &payload)?;
        self.with_state_checkpoint(|engine| {
            let effects = engine.queue_local_sibling_payload(
                chat_id,
                sibling_payload,
                &message_id,
                now,
                None,
            );
            engine.persist()?;
            Ok(ProtocolDirectSendResult {
                message_id,
                event_ids: effects
                    .iter()
                    .map(|effect| match effect {
                        ProtocolEffect::Publish(publish) => publish.event.id.to_string(),
                    })
                    .collect(),
                effects,
            })
        })
    }

    fn queue_local_sibling_payload(
        &mut self,
        chat_id: &str,
        sibling_payload: Vec<u8>,
        message_id: &str,
        now: UnixSeconds,
        eligible_devices: Option<BTreeSet<NdrDevicePubkey>>,
    ) -> Vec<ProtocolEffect> {
        let existing = self
            .pending_local_sibling_sends
            .iter()
            .position(|pending| pending.message_id == message_id && pending.chat_id == chat_id)
            .map(|index| self.pending_local_sibling_sends.remove(index));
        let mut pending = existing.unwrap_or_else(|| ProtocolPendingLocalSiblingSend {
            eligible_devices,
            chat_id: chat_id.to_string(),
            payload: sibling_payload,
            message_id: message_id.to_string(),
            completed_devices: BTreeSet::new(),
            created_at_secs: now.get(),
            next_retry_at_secs: now.get(),
        });
        let sessions = self.session_manager.clone();
        let (effects, complete) = match self
            .prepare_pending_local_sibling_send(&mut pending, NdrUnixSeconds(now.get()))
        {
            Ok(result) => result,
            Err(_) => {
                self.session_manager = sessions;
                (Vec::new(), false)
            }
        };
        if !complete {
            self.pending_local_sibling_sends.push(pending);
        }
        effects
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
        let (event_ids, effects) = self.queue_remote_payload(
            peer_pubkey,
            chat_id,
            remote_payload,
            inner_event_id,
            &message_id,
            now,
        )?;
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
            self,
            inner_event_id.clone(),
            chat_id.to_string(),
            &mut event_ids,
        )?;

        if !local.relay_gaps.is_empty() {
            anyhow::bail!(
                "direct send readiness invariant failed: local sibling relay gaps remain"
            );
        }
        self.persist()?;
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
        })
    }

    #[allow(clippy::too_many_arguments)]
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
        let (mut event_ids, mut effects) = self.queue_remote_payload(
            peer_pubkey,
            chat_id,
            remote_payload,
            inner_event_id,
            &message_id,
            now,
        )?;
        let snapshot = self.session_manager.snapshot();
        let eligible_devices = user_record_snapshot(&snapshot, self.local_owner)
            .and_then(roster_device_pubkeys)
            .unwrap_or_default()
            .into_iter()
            .filter(|device| *device != self.local_device)
            .collect();
        let sibling_effects = self.queue_local_sibling_payload(
            chat_id,
            local_sibling_payload,
            &message_id,
            now,
            Some(eligible_devices),
        );
        event_ids.extend(sibling_effects.iter().map(|effect| match effect {
            ProtocolEffect::Publish(publish) => publish.event.id.to_string(),
        }));
        effects.extend(sibling_effects);

        self.persist()?;
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
        })
    }
}
