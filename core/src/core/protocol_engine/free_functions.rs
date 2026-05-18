fn protocol_effects_from_prepared(
    prepared: &PreparedSend,
    inner_event_id: Option<String>,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<ProtocolEffect>> {
    let mut bootstrap = Vec::new();
    let mut payload = Vec::new();
    let target_owner_pubkey_hex = Some(public_owner(prepared.recipient_owner)?.to_hex());
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        bootstrap.push(ProtocolPublishEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_owner_pubkey_hex: target_owner_pubkey_hex.clone(),
            target_device_id: None,
        });
    }
    for delivery in &prepared.deliveries {
        let event = message_event(&delivery.envelope)?;
        event_ids.push(event.id.to_string());
        payload.push(ProtocolPublishEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_owner_pubkey_hex: Some(public_owner(delivery.owner_pubkey)?.to_hex()),
            target_device_id: Some(public_device(delivery.device_pubkey)?.to_hex()),
        });
    }
    Ok(protocol_publish_effects(bootstrap, payload))
}

fn protocol_effects_from_group_prepared_publish(
    prepared: &GroupPreparedPublish,
    inner_event_id: Option<String>,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<ProtocolEffect>> {
    let mut bootstrap = Vec::new();
    let mut payload = Vec::new();
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        bootstrap.push(ProtocolPublishEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_owner_pubkey_hex: None,
            target_device_id: None,
        });
    }
    for delivery in &prepared.deliveries {
        let event = message_event(&delivery.envelope)?;
        event_ids.push(event.id.to_string());
        payload.push(ProtocolPublishEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_owner_pubkey_hex: Some(public_owner(delivery.owner_pubkey)?.to_hex()),
            target_device_id: Some(public_device(delivery.device_pubkey)?.to_hex()),
        });
    }
    for sender_key_message in &prepared.sender_key_messages {
        let event = group_sender_key_message_event(sender_key_message)?;
        event_ids.push(event.id.to_string());
        payload.push(ProtocolPublishEvent {
            event,
            inner_event_id: None,
            target_owner_pubkey_hex: None,
            target_device_id: None,
        });
    }
    Ok(protocol_publish_effects(bootstrap, payload))
}

fn protocol_publish_effects(
    bootstrap: Vec<ProtocolPublishEvent>,
    payload: Vec<ProtocolPublishEvent>,
) -> Vec<ProtocolEffect> {
    if bootstrap.is_empty() {
        return payload.into_iter().map(protocol_publish_effect).collect();
    }
    if payload.is_empty() {
        return bootstrap.into_iter().map(protocol_publish_effect).collect();
    }
    vec![ProtocolEffect::PublishStagedFirstContact { bootstrap, payload }]
}

fn protocol_publish_effect(publish: ProtocolPublishEvent) -> ProtocolEffect {
    match (
        publish.inner_event_id,
        publish.target_owner_pubkey_hex,
        publish.target_device_id,
    ) {
        (None, None, None) => ProtocolEffect::PublishSigned(publish.event),
        (inner_event_id, target_owner_pubkey_hex, target_device_id) => {
            ProtocolEffect::PublishSignedForInnerEvent {
                event: publish.event,
                inner_event_id,
                target_owner_pubkey_hex,
                target_device_id,
            }
        }
    }
}

fn sort_dedup_protocol_pubkeys(pubkeys: &mut Vec<PublicKey>) {
    pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
    pubkeys.dedup();
}

fn pending_retry_delay_secs(created_at_secs: u64, now: NdrUnixSeconds) -> u64 {
    let age_secs = now.get().saturating_sub(created_at_secs);
    match age_secs {
        0..=29 => PENDING_RETRY_DELAY_SECS,
        30..=119 => 15,
        _ => 60,
    }
}

fn next_pending_retry_at_secs(created_at_secs: u64, now: NdrUnixSeconds) -> u64 {
    now.get()
        .saturating_add(pending_retry_delay_secs(created_at_secs, now))
}

fn group_publish_from_prepared_send(
    prepared: PreparedSend,
    fanout: GroupPendingFanout,
) -> GroupPreparedPublish {
    let pending_fanouts = if prepared.relay_gaps.is_empty() {
        Vec::new()
    } else {
        vec![fanout]
    };
    GroupPreparedPublish {
        deliveries: prepared.deliveries,
        invite_responses: prepared.invite_responses,
        sender_key_messages: Vec::new(),
        relay_gaps: prepared.relay_gaps,
        pending_fanouts,
    }
}

fn delivered_device_hexes(prepared: &PreparedSend) -> Vec<String> {
    let mut devices = prepared
        .deliveries
        .iter()
        .map(|delivery| delivery.device_pubkey.to_hex())
        .collect::<Vec<_>>();
    devices.sort();
    devices.dedup();
    devices
}

fn pending_reason_from_gaps(gaps: &[RelayGap]) -> ProtocolPendingReason {
    if gaps
        .iter()
        .any(|gap| matches!(gap, RelayGap::MissingRoster { .. }))
    {
        ProtocolPendingReason::MissingRoster
    } else if gaps.is_empty() {
        ProtocolPendingReason::PublishRetry
    } else {
        ProtocolPendingReason::MissingDeviceInvite
    }
}

fn collect_expected_sender_pubkeys(session: &SessionState, out: &mut HashSet<PublicKey>) {
    if let Some(current) = session.their_current_nostr_public_key {
        if let Ok(pubkey) = public_device(current) {
            out.insert(pubkey);
        }
    }
    if let Some(next) = session.their_next_nostr_public_key {
        if let Ok(pubkey) = public_device(next) {
            out.insert(pubkey);
        }
    }
    for device in session.skipped_keys.keys() {
        if let Ok(pubkey) = public_device(*device) {
            out.insert(pubkey);
        }
    }
}

fn session_state_matches_sender(session: &SessionState, sender: NdrDevicePubkey) -> bool {
    session.their_current_nostr_public_key == Some(sender)
        || session.their_next_nostr_public_key == Some(sender)
        || session.skipped_keys.contains_key(&sender)
}

fn sender_resolution_owner_matches(
    resolution: ProtocolSenderOwnerResolution,
    owner: NdrOwnerPubkey,
) -> bool {
    match resolution {
        ProtocolSenderOwnerResolution::Verified {
            owner: resolved_owner,
        }
        | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
            owner: resolved_owner,
        } => resolved_owner == owner,
        ProtocolSenderOwnerResolution::PendingOwnerClaim { claimed_owner, .. } => {
            claimed_owner == owner
        }
    }
}

fn pending_inbound_owner_hexes_from_resolution(
    resolution: ProtocolSenderOwnerResolution,
) -> (Option<String>, Option<String>) {
    match resolution {
        ProtocolSenderOwnerResolution::Verified { owner }
        | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => {
            (Some(owner.to_hex()), None)
        }
        ProtocolSenderOwnerResolution::PendingOwnerClaim {
            storage_owner,
            claimed_owner,
            ..
        } => (Some(storage_owner.to_hex()), Some(claimed_owner.to_hex())),
    }
}

fn pending_inbound_sender_pubkey(pending: &ProtocolPendingInbound) -> Option<NdrDevicePubkey> {
    if let Some(envelope) = pending.envelope.as_ref() {
        return Some(envelope.sender);
    }
    pending
        .sender_message_pubkey_hex
        .as_deref()
        .and_then(|pubkey_hex| PublicKey::parse(pubkey_hex).ok())
        .map(ndr_device)
}

fn pending_inbound_sender_pubkey_hex(pending: &ProtocolPendingInbound) -> Option<String> {
    pending_inbound_sender_pubkey(pending)
        .and_then(|sender| public_device(sender).ok())
        .map(|sender| sender.to_hex())
        .or_else(|| Some(pending.event.pubkey.to_hex()))
}

fn apply_pending_inbound_metadata(
    pending: &mut ProtocolPendingInbound,
    metadata: ProtocolPendingInboundMetadata,
) -> bool {
    let mut changed = false;
    if pending.event_id.is_empty() && !metadata.event_id.is_empty() {
        pending.event_id = metadata.event_id;
        changed = true;
    }
    if pending.envelope.is_none() && metadata.envelope.is_some() {
        pending.envelope = metadata.envelope;
        changed = true;
    }
    if pending.sender_message_pubkey_hex.is_none() && metadata.sender_message_pubkey_hex.is_some() {
        pending.sender_message_pubkey_hex = metadata.sender_message_pubkey_hex;
        changed = true;
    }
    if pending.resolved_owner_pubkey_hex.is_none() && metadata.resolved_owner_pubkey_hex.is_some() {
        pending.resolved_owner_pubkey_hex = metadata.resolved_owner_pubkey_hex;
        changed = true;
    }
    if pending.claimed_owner_pubkey_hex.is_none() && metadata.claimed_owner_pubkey_hex.is_some() {
        pending.claimed_owner_pubkey_hex = metadata.claimed_owner_pubkey_hex;
        changed = true;
    }
    if metadata.metadata_verified && !pending.metadata_verified {
        pending.metadata_verified = true;
        changed = true;
    }
    changed
}

fn provisional_owner_from_sender_pubkey(sender: NdrDevicePubkey) -> NdrOwnerPubkey {
    NdrOwnerPubkey::from_bytes(sender.to_bytes())
}

fn local_sibling_payload(conversation_owner: PublicKey, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    use base64::Engine;
    let wrapper = LocalSiblingPayload {
        protocol: LOCAL_SIBLING_PROTOCOL.to_string(),
        version: 1,
        conversation_owner: conversation_owner.to_hex(),
        payload: base64::engine::general_purpose::STANDARD.encode(payload),
    };
    Ok(serde_json::to_vec(&wrapper)?)
}

fn decode_local_sibling_payload(payload: &[u8]) -> Option<(PublicKey, Vec<u8>)> {
    use base64::Engine;
    let wrapper: LocalSiblingPayload = serde_json::from_slice(payload).ok()?;
    if wrapper.protocol != LOCAL_SIBLING_PROTOCOL || wrapper.version != 1 {
        return None;
    }
    let owner = PublicKey::parse(&wrapper.conversation_owner).ok()?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(wrapper.payload)
        .ok()?;
    Some((owner, payload))
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalSiblingPayload {
    protocol: String,
    version: u32,
    conversation_owner: String,
    payload: String,
}

fn ndr_owner(pubkey: PublicKey) -> NdrOwnerPubkey {
    NdrOwnerPubkey::from_bytes(pubkey.to_bytes())
}

fn ndr_device(pubkey: PublicKey) -> NdrDevicePubkey {
    NdrDevicePubkey::from_bytes(pubkey.to_bytes())
}

fn delivered_device_set(device_hexes: &[String]) -> HashSet<NdrDevicePubkey> {
    device_hexes
        .iter()
        .filter_map(|hex| PublicKey::parse(hex).ok())
        .map(ndr_device)
        .collect()
}

fn public_owner(pubkey: NdrOwnerPubkey) -> anyhow::Result<PublicKey> {
    Ok(PublicKey::from_slice(&pubkey.to_bytes())?)
}

fn public_device(pubkey: NdrDevicePubkey) -> anyhow::Result<PublicKey> {
    Ok(PublicKey::from_slice(&pubkey.to_bytes())?)
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
