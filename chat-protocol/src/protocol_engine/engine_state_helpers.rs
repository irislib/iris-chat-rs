/// Earlier protocol states already retained exact signed owner evidence, but
/// predated the ratchet library's separate evidence index. Feed those original
/// events into restoration so valid sessions do not become unauthorized after
/// an upgrade. Keep existing heads: the library resolves newer revocations and
/// conflicting signed events instead of trusting cached authorization flags.
fn restore_signed_peer_evidence_for_session_manager(state: &mut ProtocolEnginePersistedState) {
    for (owner, evidence) in &state.invite_owner_app_keys_evidence {
        if *owner == state.session_manager.local_owner_pubkey {
            continue;
        }
        let ProtocolAppKeysEvidence::Verified(event) = evidence else {
            continue;
        };
        if !state
            .session_manager
            .verified_peer_app_keys_events
            .iter()
            .any(|existing| existing.id == event.id)
        {
            state
                .session_manager
                .verified_peer_app_keys_events
                .push(event.as_ref().clone());
        }
    }
}

fn quarantine_unverified_owner_rosters(
    snapshot: &mut SessionManagerSnapshot,
    verified_app_keys_owners: &BTreeSet<NdrOwnerPubkey>,
) {
    for user in &mut snapshot.users {
        if verified_app_keys_owners.contains(&user.owner_pubkey) {
            continue;
        }

        // A device may always speak for the identity represented by its own
        // key. Preserve only that exact legacy singleton; every O != D binding
        // needs persisted AppKeys provenance.
        let exact_self_owned_singleton = user.roster.as_ref().is_some_and(|roster| {
            roster.devices().len() == 1
                && provisional_owner_from_sender_pubkey(roster.devices()[0].device_pubkey)
                    == user.owner_pubkey
        });
        if !exact_self_owned_singleton {
            user.roster = None;
        }

        for device in &mut user.devices {
            // O = D is authenticated by possession of D's key even when an
            // older persisted session has no synthetic singleton roster.
            device.authorized =
                provisional_owner_from_sender_pubkey(device.device_pubkey) == user.owner_pubkey;
            if !device.authorized {
                device.is_stale = false;
                device.stale_since = None;
            }
        }
    }
}

fn load_or_create_local_invite(
    storage: &dyn StorageAdapter,
    device_pubkey: PublicKey,
    device_id: &str,
    owner_pubkey: PublicKey,
) -> anyhow::Result<Invite> {
    let storage_key = format!("device-invite/{device_id}");
    if let Some(serialized) = storage.get(&storage_key)? {
        if let Ok(invite) = Invite::deserialize(&serialized) {
            return Ok(normalize_local_invite_owner(invite, owner_pubkey));
        }
    }

    let mut invite = Invite::create_new(device_pubkey, Some(device_id.to_string()), None)?;
    invite = normalize_local_invite_owner(invite, owner_pubkey);
    storage.put(&storage_key, invite.serialize()?)?;
    Ok(invite)
}

fn normalize_local_invite_owner(mut invite: Invite, owner_pubkey: PublicKey) -> Invite {
    if invite.metadata_privacy_version != nostr_double_ratchet::METADATA_PRIVACY_VERSION {
        invite.metadata_privacy_version = nostr_double_ratchet::METADATA_PRIVACY_VERSION;
        invite.created_at = NdrUnixSeconds(
            unix_now()
                .get()
                .max(invite.created_at.get().saturating_add(1)),
        );
    }
    invite.inviter_owner_pubkey = Some(ndr_owner(owner_pubkey));
    invite.owner_public_key = Some(owner_pubkey);
    invite
}

fn user_record_snapshot(
    snapshot: &SessionManagerSnapshot,
    owner: NdrOwnerPubkey,
) -> Option<&UserRecordSnapshot> {
    snapshot
        .users
        .iter()
        .find(|user| user.owner_pubkey == owner)
}

fn roster_device_pubkeys(user: &UserRecordSnapshot) -> Option<Vec<NdrDevicePubkey>> {
    user.roster.as_ref().map(|roster| {
        roster
            .devices()
            .iter()
            .map(|entry| entry.device_pubkey)
            .collect()
    })
}

fn user_can_send_to_device(user: &UserRecordSnapshot, device: NdrDevicePubkey) -> bool {
    user.devices
        .iter()
        .find(|record| record.device_pubkey == device)
        .is_some_and(device_record_can_send)
}

fn device_record_can_send(record: &DeviceRecordSnapshot) -> bool {
    record.authorized
        && !record.is_stale
        && (record.active_session.is_some()
            || !record.inactive_sessions.is_empty()
            || record.public_invite.is_some())
}

impl ProtocolEngine {
    /// Original signed evidence for authenticated own-device reconciliation.
    pub fn authorization_events(&self) -> Vec<Event> {
        let mut events: BTreeMap<nostr::EventId, Event> = BTreeMap::new();
        for evidence in self.invite_owner_app_keys_evidence.values() {
            if let ProtocolAppKeysEvidence::Verified(event) = evidence {
                events.insert(event.id, event.as_ref().clone());
            }
        }
        for history in self.group_roster_fact_histories.values() {
            for event in &history.events {
                events.insert(event.id, event.clone());
            }
        }
        events.into_values().collect()
    }
}

impl ProtocolEngine {
    fn with_group_fanout_checkpoint<T>(
        &mut self,
        operation: impl FnOnce(&mut Self, &mut GroupFanoutRetryUndo) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        // Fanout retries mutate only sessions, selected targets, and ready
        // publications. Keep unrelated queues and deferred fanouts installed.
        // Persistence is the final fallible step: after a successful save its
        // delivery-journal cleanup cannot need an in-memory rollback.
        let checkpoint = (
            self.session_manager.clone(),
            self.ready_group_publications.clone(),
            self.batch_persist_dirty.get(),
        );
        let mut undo = GroupFanoutRetryUndo::default();
        let result = operation(self, &mut undo);
        if result.is_err() {
            undo.restore(&mut self.pending_group_fanouts);
            self.session_manager = checkpoint.0;
            self.ready_group_publications = checkpoint.1;
            self.batch_persist_dirty.set(checkpoint.2);
        }
        self.invalidate_known_message_author_cache();
        result
    }

    fn state_checkpoint(&self) -> ProtocolEngineCheckpoint {
        ProtocolEngineCheckpoint {
            session_manager: self.session_manager.clone(),
            group_manager: self.group_manager.clone(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            ready_group_publications: self.ready_group_publications.clone(),
            pending_local_sibling_sends: self.pending_local_sibling_sends.clone(),
            pending_remote_sends: self.pending_remote_sends.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            processed_group_sender_key_messages: self.processed_group_sender_key_messages.clone(),
            group_sender_key_retry: self.group_sender_key_retry.borrow().clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            verified_app_keys_owners: self.verified_app_keys_owners.clone(),
            invite_owner_app_keys_evidence: self.invite_owner_app_keys_evidence.clone(),
            processed_private_invite_response_ids: self
                .processed_private_invite_response_ids
                .clone(),
            group_invite_uses: self.group_invite_uses.clone(),
            local_app_keys_observed: self.local_app_keys_observed,
            subscription_generation: self.subscription_generation,
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: ProtocolEngineCheckpoint) {
        self.group_invite_uses = checkpoint.group_invite_uses;
        self.session_manager = checkpoint.session_manager;
        self.group_manager = checkpoint.group_manager;
        self.invalidate_pending_inbound_retry_deadline();
        self.pending_inbound = checkpoint.pending_inbound;
        self.pending_group_fanouts = checkpoint.pending_group_fanouts;
        self.ready_group_publications = checkpoint.ready_group_publications;
        self.pending_local_sibling_sends = checkpoint.pending_local_sibling_sends;
        self.pending_remote_sends = checkpoint.pending_remote_sends;
        self.pending_group_pairwise_payloads = checkpoint.pending_group_pairwise_payloads;
        self.pending_group_sender_key_messages = checkpoint.pending_group_sender_key_messages;
        self.pending_group_sender_key_repairs = checkpoint.pending_group_sender_key_repairs;
        self.processed_group_sender_key_messages = checkpoint.processed_group_sender_key_messages;
        self.group_sender_key_retry
            .replace(checkpoint.group_sender_key_retry);
        self.answered_group_sender_key_repairs = checkpoint.answered_group_sender_key_repairs;
        self.pending_decrypted_deliveries = checkpoint.pending_decrypted_deliveries;
        self.group_roster_fact_histories = checkpoint.group_roster_fact_histories;
        self.verified_app_keys_owners = checkpoint.verified_app_keys_owners;
        self.invite_owner_app_keys_evidence = checkpoint.invite_owner_app_keys_evidence;
        self.processed_private_invite_response_ids =
            checkpoint.processed_private_invite_response_ids;
        self.local_app_keys_observed = checkpoint.local_app_keys_observed;
        self.subscription_generation = checkpoint.subscription_generation;
        self.invalidate_known_message_author_cache();
    }

    fn with_state_checkpoint<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let checkpoint = self.state_checkpoint();
        match operation(self) {
            Ok(value) => {
                self.invalidate_known_message_author_cache();
                Ok(value)
            }
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }
}
