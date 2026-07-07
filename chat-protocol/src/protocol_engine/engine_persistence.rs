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

    fn persist(&self) -> anyhow::Result<()> {
        if self.batch_depth.get() > 0 {
            self.batch_persist_dirty.set(true);
            return Ok(());
        }
        self.persist_now()
    }

    fn persist_now(&self) -> anyhow::Result<()> {
        let state = ProtocolEnginePersistedState {
            version: PROTOCOL_ENGINE_STATE_VERSION,
            session_manager: self.session_manager.snapshot(),
            group_manager: self.group_manager.snapshot(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            delivered_group_sender_key_acks: self.delivered_group_sender_key_acks.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            subscription_generation: self.subscription_generation,
        };
        self.batch_persist_dirty.set(false);
        self.storage
            .put(PROTOCOL_ENGINE_STATE_KEY, serde_json::to_string(&state)?)?;
        Ok(())
    }

    pub fn enter_batch(&self) {
        self.batch_depth
            .set(self.batch_depth.get().saturating_add(1));
    }

    pub fn exit_batch(&self) -> anyhow::Result<()> {
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
