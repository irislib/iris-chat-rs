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

    fn persist(&mut self) -> anyhow::Result<()> {
        if self.batch_depth.get() > 0 {
            self.batch_persist_dirty.set(true);
            return Ok(());
        }
        self.persist_now()
    }

    fn persist_now(&mut self) -> anyhow::Result<()> {
        let state = ProtocolEnginePersistedState {
            version: PROTOCOL_ENGINE_STATE_VERSION,
            session_manager: self.session_manager.snapshot(),
            group_manager: self.group_manager.snapshot(),
            verified_app_keys_owners: self.verified_app_keys_owners.clone(),
            app_keys_provenance_version: PROTOCOL_APP_KEYS_PROVENANCE_VERSION,
            invite_owner_app_keys_evidence: self.invite_owner_app_keys_evidence.clone(),
            processed_private_invite_response_ids: self
                .processed_private_invite_response_ids
                .clone(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            pending_local_sibling_sends: self.pending_local_sibling_sends.clone(),
            pending_remote_sends: self.pending_remote_sends.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            processed_group_sender_key_messages: self.processed_group_sender_key_messages.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.iter()
                .filter(|delivery| !delivery.discarded).cloned().collect(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            subscription_generation: self.subscription_generation,
        };
        let pending = self.pending_group_sender_key_messages.serialized()?;
        let (json, layout) = layout_protocol_checkpoint_with_pending(
            serde_json::to_string(&state)?,
            &self.checkpoint_layout.borrow(),
            Some(&pending),
        );
        self.storage.put(PROTOCOL_ENGINE_STATE_KEY, json)?;
        self.batch_persist_dirty.set(false);
        self.pending_decrypted_deliveries.retain(|delivery| !delivery.discarded);
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
