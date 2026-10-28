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
        let now = unix_now();
        let session_change_possible = match source_event {
            Some(event) if owner_pubkey != self.owner_pubkey => self
                .session_manager
                .peer_app_keys_event_would_change(event, NdrUnixSeconds(now.get()))?,
            _ => true,
        };
        let previous_session = session_change_possible.then(|| self.session_manager.clone());
        let previous_evidence = self.invite_owner_app_keys_evidence.get(&owner).cloned();
        let previously_verified = self.verified_app_keys_owners.contains(&owner);
        let previously_observed = self.local_app_keys_observed;
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
            if session_change_possible {
                self.session_manager
                    .observe_peer_app_keys_event(event.clone(), NdrUnixSeconds(now.get()))?
            } else {
                false
            }
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
            roster_changed = self.session_manager.snapshot()
                != previous_session
                    .as_ref()
                    .expect("local and projected roster checkpoint")
                    .snapshot();
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
        if let Some(previous_session) = previous_session {
            checkpoint.session_manager = previous_session;
        }
        checkpoint.local_app_keys_observed = previously_observed;
        if !previously_verified {
            checkpoint.verified_app_keys_owners.remove(&owner);
        }
        match previous_evidence {
            Some(evidence) => {
                checkpoint
                    .invite_owner_app_keys_evidence
                    .insert(owner, evidence);
            }
            None => {
                checkpoint.invite_owner_app_keys_evidence.remove(&owner);
            }
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

}
