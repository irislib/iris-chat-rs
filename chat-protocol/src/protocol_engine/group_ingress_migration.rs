fn is_synthetic_group_delivery(delivery: &ProtocolPendingDecryptedDelivery) -> bool {
    delivery
        .event_id
        .as_deref()
        .is_some_and(|id| id.starts_with("group-delivery:"))
}

fn quarantine_legacy_group_ingress(
    storage: &dyn StorageAdapter,
    raw: &str,
    state: &mut ProtocolEnginePersistedState,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.group_ingress_provenance_version <= PROTOCOL_GROUP_INGRESS_PROVENANCE_VERSION,
        "Saved group ingress uses an unsupported version. Existing data has been kept."
    );
    if state.group_ingress_provenance_version == PROTOCOL_GROUP_INGRESS_PROVENANCE_VERSION {
        return Ok(());
    }
    let has_legacy_ingress = !state.pending_group_sender_key_messages.is_empty()
        || !state.pending_group_sender_key_repairs.is_empty()
        || state
            .pending_decrypted_deliveries
            .iter()
            .any(is_synthetic_group_delivery);
    if has_legacy_ingress {
        // get() reconstructs segmented/checkpoint storage. Retain its exact
        // original bytes once, outside executable state, before removing any
        // unproven input. A failed archive write leaves the old state intact;
        // a failed subsequent checkpoint can safely repeat this migration.
        if let Some(previous) = storage.get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)? {
            anyhow::ensure!(
                previous == raw,
                "A different legacy group state is already quarantined. Existing data has been kept."
            );
        }
        // A previous preservation attempt may have written the bytes but failed
        // synchronization. Matching bytes alone are not a durable acknowledgment.
        storage.put_preserved(LEGACY_GROUP_INGRESS_QUARANTINE_KEY, raw.to_owned())?;
        state.pending_group_sender_key_messages = PendingGroupMessages::default();
        state.pending_group_sender_key_repairs.clear();
        state
            .pending_decrypted_deliveries
            .retain(|delivery| !is_synthetic_group_delivery(delivery));
    }
    // The normal startup checkpoint commits this marker with the cleared
    // queues. Outgoing fanouts, group state/history and ordinary DM journals
    // retain their existing values; none acquire invented ingress provenance.
    state.group_ingress_provenance_version = PROTOCOL_GROUP_INGRESS_PROVENANCE_VERSION;
    Ok(())
}
