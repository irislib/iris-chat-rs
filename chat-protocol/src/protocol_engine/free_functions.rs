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
    for (index, delivery) in prepared.deliveries.iter().enumerate() {
        let event = message_event_for_delivery_with_offset(delivery, index as u64)?;
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
    let sender_key_offset = prepared.deliveries.len() as u64;
    for (index, sender_key_message) in prepared.sender_key_messages.iter().enumerate() {
        let mut sender_key_message = sender_key_message.clone();
        sender_key_message.created_at = NdrUnixSeconds(
            sender_key_message
                .created_at
                .get()
                .saturating_add(sender_key_offset)
                .saturating_add(index as u64),
        );
        let event = group_sender_key_message_event(&sender_key_message)?;
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
    message_event_for_delivery_with_offset(delivery, 0)
}

fn message_event_for_delivery_with_offset(
    delivery: &Delivery,
    created_at_offset_secs: u64,
) -> anyhow::Result<Event> {
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
    .custom_created_at(Timestamp::from(
        envelope
            .created_at
            .get()
            .saturating_add(created_at_offset_secs),
    ))
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
