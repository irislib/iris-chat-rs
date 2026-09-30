impl ProtocolEngine {
    pub fn session_manager_snapshot_for_test(&self) -> SessionManagerSnapshot {
        self.session_manager.snapshot()
    }

    pub fn group_manager_snapshot_for_test(&self) -> GroupManagerSnapshot {
        self.group_manager.snapshot()
    }

    pub fn pending_inbound_for_test(&self) -> Vec<ProtocolPendingInboundTestDebug> {
        self.pending_inbound
            .iter()
            .map(|pending| ProtocolPendingInboundTestDebug {
                event_id: if pending.event_id.is_empty() {
                    pending.event.id.to_string()
                } else {
                    pending.event_id.clone()
                },
                sender_message_pubkey_hex: pending.sender_message_pubkey_hex.clone(),
                claimed_owner_pubkey_hex: pending.claimed_owner_pubkey_hex.clone(),
                has_envelope: pending.envelope.is_some(),
                metadata_verified: pending.metadata_verified,
            })
            .collect()
    }

    pub fn known_message_author_cache_build_count_for_test(&self) -> u64 {
        self.known_message_author_cache_build_count.get()
    }

    pub fn pending_decrypted_deliveries_len_for_test(&self) -> usize {
        self.pending_decrypted_deliveries.len()
    }
}
