fn protocol_effects_from_prepared(
    prepared: &PreparedSend,
    inner_event_id: Option<String>,
    chat_id: String,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<ProtocolEffect>> {
    let mut publishes = Vec::new();
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        publishes.push(ProtocolPublish {
            event,
            chat_id: chat_id.clone(),
            inner_event_id: None,
        });
    }
    for delivery in &prepared.deliveries {
        let event = message_event_for_delivery(delivery)?;
        event_ids.push(event.id.to_string());
        let publish = ProtocolPublish {
            event,
            chat_id: chat_id.clone(),
            inner_event_id: inner_event_id.clone(),
        };
        publishes.push(publish);
    }
    Ok(publishes.into_iter().map(ProtocolEffect::Publish).collect())
}

fn protocol_effects_from_group_prepared_publish(
    prepared: &GroupPreparedPublish,
    inner_event_id: Option<String>,
    chat_id: String,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<ProtocolEffect>> {
    let mut publishes = Vec::new();
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        publishes.push(ProtocolPublish {
            event,
            chat_id: chat_id.clone(),
            inner_event_id: None,
        });
    }
    for delivery in &prepared.deliveries {
        let event = message_event_for_delivery(delivery)?;
        event_ids.push(event.id.to_string());
        let publish = ProtocolPublish {
            event,
            chat_id: chat_id.clone(),
            inner_event_id: inner_event_id.clone(),
        };
        publishes.push(publish);
    }
    for sender_key_message in &prepared.sender_key_messages {
        let event = group_sender_key_message_event(sender_key_message)?;
        event_ids.push(event.id.to_string());
        publishes.push(ProtocolPublish {
            event,
            chat_id: chat_id.clone(),
            inner_event_id: inner_event_id.clone(),
        });
    }
    Ok(publishes.into_iter().map(ProtocolEffect::Publish).collect())
}

fn message_event_for_delivery(delivery: &Delivery) -> anyhow::Result<Event> {
    let envelope = &delivery.envelope;
    let author_secret_key = nostr::SecretKey::from_slice(&envelope.signer_secret_key)?;
    let author_keys = Keys::new(author_secret_key);
    let derived_sender = NdrDevicePubkey::from_bytes(author_keys.public_key().to_bytes());
    if derived_sender != envelope.sender {
        anyhow::bail!("sender does not match signer secret");
    }

    let recipient = public_device_pubkey(delivery.device_pubkey)?;
    let recipient_hex = recipient.to_hex();
    let unsigned = nostr::EventBuilder::new(
        Kind::from(MESSAGE_EVENT_KIND as u16),
        envelope.ciphertext.clone(),
    )
    .tag(nostr::Tag::parse([
        "header",
        envelope.encrypted_header.as_str(),
    ])?)
    .tag(nostr::Tag::parse(["p", recipient_hex.as_str()])?)
    .custom_created_at(Timestamp::from(envelope.created_at.get()))
    .build(public_device_pubkey(envelope.sender)?);

    Ok(unsigned.sign_with_keys(&author_keys)?)
}

fn public_device_pubkey(device_pubkey: NdrDevicePubkey) -> anyhow::Result<PublicKey> {
    Ok(PublicKey::from_slice(&device_pubkey.to_bytes())?)
}

fn classify_group_pairwise_payload(payload: &[u8]) -> anyhow::Result<(bool, bool)> {
    let codec = JsonGroupPayloadCodecV1;
    let Some(command) = codec.decode_pairwise_command(payload)? else {
        return Ok((false, false));
    };
    let supported = match command {
        GroupPairwiseCommand::MetadataSnapshot { snapshot } => snapshot.protocol.is_sender_key_v1(),
        GroupPairwiseCommand::SenderKeyDistribution { .. }
        | GroupPairwiseCommand::SenderKeyRepairRequest { .. } => true,
        _ => false,
    };
    Ok((true, supported))
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
    local_sibling_payload_with_original_sender(conversation_owner, None, payload)
}

fn local_sibling_payload_with_original_sender(
    conversation_owner: PublicKey,
    original_sender_device: Option<PublicKey>,
    payload: &[u8],
) -> anyhow::Result<Vec<u8>> {
    use base64::Engine;
    let wrapper = LocalSiblingPayload {
        protocol: LOCAL_SIBLING_PROTOCOL.to_string(),
        version: 1,
        conversation_owner: conversation_owner.to_hex(),
        original_sender_device: original_sender_device.map(|pubkey| pubkey.to_hex()),
        payload: base64::engine::general_purpose::STANDARD.encode(payload),
    };
    Ok(serde_json::to_vec(&wrapper)?)
}

fn decode_local_sibling_payload(payload: &[u8]) -> Option<(PublicKey, Option<PublicKey>, Vec<u8>)> {
    use base64::Engine;
    let wrapper: LocalSiblingPayload = serde_json::from_slice(payload).ok()?;
    if wrapper.protocol != LOCAL_SIBLING_PROTOCOL || wrapper.version != 1 {
        return None;
    }
    let owner = PublicKey::parse(&wrapper.conversation_owner).ok()?;
    let original_sender_device = match wrapper.original_sender_device {
        Some(device) => Some(PublicKey::parse(&device).ok()?),
        None => None,
    };
    let payload = base64::engine::general_purpose::STANDARD
        .decode(wrapper.payload)
        .ok()?;
    Some((owner, original_sender_device, payload))
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalSiblingPayload {
    protocol: String,
    version: u32,
    conversation_owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_sender_device: Option<String>,
    payload: String,
}

fn ndr_owner(pubkey: PublicKey) -> NdrOwnerPubkey {
    NdrOwnerPubkey::from_bytes(pubkey.to_bytes())
}

fn ndr_device(pubkey: PublicKey) -> NdrDevicePubkey {
    NdrDevicePubkey::from_bytes(pubkey.to_bytes())
}

fn protocol_event_has_tag(event: &Event, name: &str) -> bool {
    event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some(name))
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
