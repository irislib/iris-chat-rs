impl ProtocolEngine {
    fn has_authoritative_local_roster(&self) -> bool {
        if self.local_app_keys_observed {
            return true;
        }
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == self.local_owner)
            .and_then(|user| user.roster)
            .is_some_and(|roster| {
                let devices = roster.devices();
                !devices.is_empty()
                    && (devices.len() > 1 || devices[0].device_pubkey != self.local_device)
            })
    }

    #[cfg_attr(all(test, target_os = "linux"), track_caller)]
    fn persist(&mut self) -> anyhow::Result<()> {
        #[cfg(all(test, target_os = "linux"))]
        tests::audit_retry_persist_requested(std::panic::Location::caller());
        if self.batch_depth.get() > 0 {
            self.batch_persist_dirty.set(true);
            return Ok(());
        }
        self.persist_now()
    }

    #[cfg_attr(all(test, target_os = "linux"), track_caller)]
    fn persist_now(&mut self) -> anyhow::Result<()> {
        let state = ProtocolEnginePersistedState {
            version: PROTOCOL_ENGINE_STATE_VERSION,
            group_invite_uses: self.group_invite_uses.clone(),
            session_manager: self.session_manager.snapshot(),
            group_manager: self.group_manager.snapshot(),
            verified_app_keys_owners: self.verified_app_keys_owners.clone(),
            app_keys_provenance_version: PROTOCOL_APP_KEYS_PROVENANCE_VERSION,
            invite_owner_app_keys_evidence: self.invite_owner_app_keys_evidence.clone(),
            processed_private_invite_response_ids: self
                .processed_private_invite_response_ids
                .clone(),
            pending_inbound: Vec::new(),
            pending_group_fanouts: Vec::new(),
            ready_group_publications: self.ready_group_publications.clone(),
            pending_local_sibling_sends: self.pending_local_sibling_sends.clone(),
            pending_remote_sends: self.pending_remote_sends.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: PendingGroupMessages::default(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            processed_group_sender_key_messages: self.processed_group_sender_key_messages.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self
                .pending_decrypted_deliveries
                .iter()
                .filter(|delivery| !delivery.discarded)
                .cloned()
                .collect(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            subscription_generation: self.subscription_generation,
        };
        let (checkpoint, layout) = record_protocol_checkpoint(
            state,
            &self.checkpoint_layout.borrow(),
            &self.pending_group_sender_key_messages,
            &self.pending_group_fanouts,
            &self.pending_inbound,
        )?;
        self.storage
            .put_checkpoint(PROTOCOL_ENGINE_STATE_KEY, &checkpoint)?;
        #[cfg(all(test, target_os = "linux"))]
        tests::audit_retry_persisted(std::panic::Location::caller(), self.batch_depth.get());
        self.batch_persist_dirty.set(false);
        self.pending_decrypted_deliveries
            .retain(|delivery| !delivery.discarded);
        *self.checkpoint_layout.borrow_mut() = layout;
        Ok(())
    }

    pub fn enter_batch(&self) {
        if self.batch_depth.get() == 0 {
            self.group_sender_key_retry.borrow_mut().reset_budget();
        }
        self.batch_depth
            .set(self.batch_depth.get().saturating_add(1));
    }

    pub fn exit_batch(&mut self) -> anyhow::Result<()> {
        let depth = self.batch_depth.get();
        if depth == 0 {
            return Ok(());
        }
        self.batch_depth.set(depth - 1);
        if self.batch_depth.get() == 0 && self.batch_persist_dirty.get() {
            self.persist_now()?;
        }
        Ok(())
    }
}

fn record_protocol_checkpoint(
    core: ProtocolEnginePersistedState,
    previous: &ProtocolCheckpointLayout,
    pending: &PendingGroupMessages,
    fanouts: &[ProtocolPendingGroupFanout],
    inbound: &[ProtocolPendingInbound],
) -> anyhow::Result<(crate::StorageCheckpoint, ProtocolCheckpointLayout)> {
    let core_json = serde_json::to_string(&core)?;
    let empty_prefix = format!(
        "{{\"version\":{},\"pending_group_sender_key_messages\":[],\"pending_group_fanouts\":[],\"pending_inbound\":[]",
        PROTOCOL_ENGINE_STATE_VERSION
    );
    // These are the first four fields of our own serializer, not untrusted
    // substring matches. A schema/order change must update this contract.
    anyhow::ensure!(
        core_json.starts_with(&empty_prefix),
        "unexpected checkpoint core prefix"
    );
    let core_previous = previous.core.as_deref();
    let empty_layout = ProtocolCheckpointLayout::default();
    let (core_json, core_layout) = layout_protocol_checkpoint_with_cached_fields(
        core_json,
        core_previous.unwrap_or(&empty_layout),
        None,
        &[],
    );
    let tail = core_json
        .strip_prefix(&empty_prefix)
        .ok_or_else(|| anyhow::anyhow!("unexpected checkpoint layout prefix"))?;
    let mut value = crate::StorageCheckpoint::new();
    value.push_inline(
        "prefix",
        format!(
            "{{\"version\":{},\"pending_group_sender_key_messages\":[",
            PROTOCOL_ENGINE_STATE_VERSION
        ),
        0,
    );
    for (index, body) in pending.record_bodies()?.into_iter().enumerate() {
        value.push_record(
            0,
            body,
            if index == 0 { "" } else { "," }.into(),
            String::new(),
            0,
        );
    }
    value.push_inline("between", "],\"pending_group_fanouts\":[".into(), 0);
    #[derive(Serialize)]
    struct FanoutTail<'a> {
        remaining_devices: &'a Option<BTreeSet<NdrDevicePubkey>>,
        inner_event_id: &'a Option<String>,
        created_at_secs: u64,
        next_retry_at_secs: u64,
    }
    for (index, pending) in fanouts.iter().enumerate() {
        let suffix = serde_json::to_string(&FanoutTail {
            remaining_devices: &pending.remaining_devices,
            inner_event_id: &pending.inner_event_id,
            created_at_secs: pending.created_at_secs,
            next_retry_at_secs: pending.next_retry_at_secs,
        })?;
        let prefix = format!(
            "{}{{\"group_id\":{},\"fanout\":",
            if index == 0 { "" } else { "," },
            serde_json::to_string(&pending.group_id)?
        );
        value.push_record(
            1,
            pending.fanout.checkpoint_body()?,
            prefix,
            format!(",{}", &suffix[1..]),
            0,
        );
    }
    value.push_inline("before_inbound", "],\"pending_inbound\":[".into(), 0);
    append_checkpoint_inbound(&mut value, inbound)?;
    value.push_inline("core", format!("]{tail}"), 0);
    let compact_bytes = value.len();
    if compact_bytes < CHECKPOINT_RESERVE {
        return Ok((value, ProtocolCheckpointLayout::default()));
    }
    // Padding belongs to the record, not its old ordinal. Append/removal
    // cannot dirty the descriptors of every surviving backlog record.
    value.pad_records(CHECKPOINT_PAGE, CHECKPOINT_PAGE / 2);
    let reuse = previous.bytes <= compact_bytes.saturating_mul(3) + CHECKPOINT_RESERVE;
    let target = if reuse && value.len() <= previous.bytes {
        previous.bytes
    } else {
        value.len().div_ceil(CHECKPOINT_RESERVE) * CHECKPOINT_RESERVE
    };
    value.push_inline("reserve", String::new(), target - value.len());
    let layout = ProtocolCheckpointLayout {
        bytes: value.len(),
        record_ends: Vec::new(),
        core: Some(Box::new(core_layout)),
    };
    Ok((value, layout))
}
