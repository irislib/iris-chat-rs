fn is_synthetic_group_delivery(delivery: &ProtocolPendingDecryptedDelivery) -> bool {
    delivery
        .event_id
        .as_deref()
        .is_some_and(|id| id.starts_with("group-delivery:"))
}

fn legacy_group_ingress_quarantine(
    raw: &str,
    state: &ProtocolEnginePersistedState,
) -> anyhow::Result<String> {
    use serde_json::value::RawValue;

    #[derive(Deserialize)]
    struct LegacyQueues<'a> {
        #[serde(default, borrow)]
        pending_group_sender_key_messages: Option<&'a RawValue>,
        #[serde(default, borrow)]
        pending_group_sender_key_repairs: Option<&'a RawValue>,
        #[serde(default, borrow)]
        pending_decrypted_deliveries: Vec<&'a RawValue>,
    }
    #[derive(Serialize)]
    struct RemovedQueues<'a> {
        pending_group_sender_key_messages: &'a RawValue,
        pending_group_sender_key_repairs: &'a RawValue,
        pending_decrypted_deliveries: Vec<&'a RawValue>,
    }

    let queues: LegacyQueues<'_> = serde_json::from_str(raw)?;
    anyhow::ensure!(
        queues.pending_decrypted_deliveries.len() == state.pending_decrypted_deliveries.len(),
        "Saved group journal could not be preserved. Existing data has been kept."
    );
    let empty: &RawValue = serde_json::from_str("[]")?;
    // Preserve removed collections/items verbatim, including their original
    // JSON encodings. Never retain session/group snapshots or outgoing work:
    // those contain live keys whose later retirement must remain effective.
    Ok(serde_json::to_string(&RemovedQueues {
        pending_group_sender_key_messages: queues
            .pending_group_sender_key_messages
            .unwrap_or(empty),
        pending_group_sender_key_repairs: queues.pending_group_sender_key_repairs.unwrap_or(empty),
        pending_decrypted_deliveries: queues
            .pending_decrypted_deliveries
            .into_iter()
            .zip(&state.pending_decrypted_deliveries)
            .filter_map(|(raw, delivery)| is_synthetic_group_delivery(delivery).then_some(raw))
            .collect(),
    })?)
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
        let quarantine = legacy_group_ingress_quarantine(raw, state)?;
        // get() reconstructs segmented/checkpoint storage. Preserve only the
        // removed input before changing executable state. A failed archive or
        // subsequent checkpoint leaves the migration safe to repeat.
        if let Some(previous) = storage.get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)? {
            anyhow::ensure!(
                previous == quarantine,
                "A different legacy group state is already quarantined. Existing data has been kept."
            );
        }
        // A previous preservation attempt may have written the bytes but failed
        // synchronization. Matching bytes alone are not a durable acknowledgment.
        storage.put_preserved(LEGACY_GROUP_INGRESS_QUARANTINE_KEY, quarantine)?;
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
