use super::*;

pub(super) fn resolve_message_sender_owner(
    session_manager: &SessionManager,
    envelope: &MessageEnvelope,
    now: UnixSeconds,
) -> Option<OwnerPubkey> {
    let owners: Vec<OwnerPubkey> = session_manager
        .snapshot()
        .users
        .into_iter()
        .map(|user| user.owner_pubkey)
        .collect();

    for owner in owners {
        let mut candidate = session_manager.clone();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        match candidate.receive(&mut ctx, owner, envelope) {
            Ok(Some(_)) => return Some(owner),
            Ok(None) => {}
            Err(_) => {}
        }
    }

    None
}

pub(super) fn encode_app_direct_message_payload(
    chat_id: &str,
    message_id: &str,
    body: &str,
) -> anyhow::Result<Vec<u8>> {
    let (normalized_chat_id, _) = parse_peer_input(chat_id)?;
    Ok(serde_json::to_vec(&AppDirectMessagePayload {
        version: APP_DIRECT_MESSAGE_PAYLOAD_VERSION,
        chat_id: normalized_chat_id,
        message_id: Some(message_id.to_string()),
        body: body.to_string(),
    })?)
}

pub(super) fn encode_app_group_message_payload(
    message_id: &str,
    body: &str,
) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AppGroupMessagePayload {
        version: APP_GROUP_MESSAGE_PAYLOAD_VERSION,
        message_id: Some(message_id.to_string()),
        body: body.to_string(),
    })?)
}

pub(super) fn encode_app_control_payload(
    control_type: AppControlType,
    chat_id: Option<String>,
    message_ids: Vec<String>,
    message_ttl_seconds: Option<u64>,
) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AppControlPayload {
        version: APP_DIRECT_MESSAGE_PAYLOAD_VERSION,
        control_type,
        chat_id,
        message_ids,
        message_ttl_seconds,
    })?)
}

pub(super) fn decode_app_payload(payload: &[u8]) -> AppPayload {
    if let Some(control) = decode_app_control_payload(payload) {
        return AppPayload::Control(control);
    }
    if let Some(direct) = decode_app_direct_message_payload(payload) {
        return AppPayload::DirectMessage(direct);
    }
    if let Some(group) = decode_app_group_message_payload(payload) {
        return AppPayload::GroupMessage(group);
    }
    AppPayload::LegacyText(String::from_utf8_lossy(payload).into_owned())
}

fn decode_app_direct_message_payload(payload: &[u8]) -> Option<AppDirectMessagePayload> {
    let decoded = serde_json::from_slice::<AppDirectMessagePayload>(payload).ok()?;
    if decoded.version != APP_DIRECT_MESSAGE_PAYLOAD_VERSION {
        return None;
    }
    Some(decoded)
}

fn decode_app_control_payload(payload: &[u8]) -> Option<AppControlPayload> {
    let decoded = serde_json::from_slice::<AppControlPayload>(payload).ok()?;
    if decoded.version != APP_DIRECT_MESSAGE_PAYLOAD_VERSION {
        return None;
    }
    Some(decoded)
}

pub(super) fn is_retryable_group_payload_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("create group sender must match created_by")
        || message.contains("unknown group")
        || message.contains("revision mismatch")
}

pub(super) fn is_unknown_group_payload_error(error: &anyhow::Error) -> bool {
    error.to_string().contains("unknown group")
}

fn decode_app_group_message_payload(payload: &[u8]) -> Option<AppGroupMessagePayload> {
    let decoded = serde_json::from_slice::<AppGroupMessagePayload>(payload).ok()?;
    if decoded.version != APP_GROUP_MESSAGE_PAYLOAD_VERSION {
        return None;
    }
    Some(decoded)
}

pub(super) fn is_group_chat_id(chat_id: &str) -> bool {
    chat_id.starts_with(GROUP_CHAT_PREFIX)
}

pub(super) fn group_chat_id(group_id: &str) -> String {
    format!("{GROUP_CHAT_PREFIX}{group_id}")
}

pub(super) fn parse_group_id_from_chat_id(chat_id: &str) -> Option<String> {
    chat_id
        .strip_prefix(GROUP_CHAT_PREFIX)
        .map(|group_id| group_id.to_string())
}

pub(super) fn normalize_group_id(value: &str) -> Option<String> {
    if let Some(group_id) = parse_group_id_from_chat_id(value) {
        if !group_id.trim().is_empty() {
            return Some(group_id);
        }
        return None;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn chat_kind_for_id(chat_id: &str) -> ChatKind {
    if is_group_chat_id(chat_id) {
        ChatKind::Group
    } else {
        ChatKind::Direct
    }
}

pub(super) fn collect_expected_senders(session: &SessionState, out: &mut HashSet<String>) {
    if let Some(current) = session.their_current_nostr_public_key {
        out.insert(current.to_string());
    }
    if let Some(next) = session.their_next_nostr_public_key {
        out.insert(next.to_string());
    }
    out.extend(session.skipped_keys.keys().map(ToString::to_string));
}

pub(super) fn pending_reason_from_prepared(
    prepared: &nostr_double_ratchet::PreparedSend,
) -> Option<PendingSendReason> {
    if prepared
        .relay_gaps
        .iter()
        .any(|gap| matches!(gap, RelayGap::MissingRoster { .. }))
    {
        return Some(PendingSendReason::MissingRoster);
    }
    if prepared
        .relay_gaps
        .iter()
        .any(|gap| matches!(gap, RelayGap::MissingDeviceInvite { .. }))
    {
        return Some(PendingSendReason::MissingDeviceInvite);
    }
    if prepared.deliveries.is_empty() && prepared.invite_responses.is_empty() {
        return Some(PendingSendReason::MissingDeviceInvite);
    }
    None
}

pub(super) fn pending_reason_from_group_prepared(
    prepared: &nostr_double_ratchet::GroupPreparedSend,
) -> Option<PendingSendReason> {
    if prepared
        .remote
        .relay_gaps
        .iter()
        .any(|gap| matches!(gap, RelayGap::MissingRoster { .. }))
    {
        return Some(PendingSendReason::MissingRoster);
    }
    if prepared
        .remote
        .relay_gaps
        .iter()
        .any(|gap| matches!(gap, RelayGap::MissingDeviceInvite { .. }))
    {
        return Some(PendingSendReason::MissingDeviceInvite);
    }
    if prepared.remote.deliveries.is_empty() && prepared.remote.invite_responses.is_empty() {
        return Some(PendingSendReason::MissingDeviceInvite);
    }
    None
}

pub(super) fn build_prepared_publish_batch(
    prepared: &nostr_double_ratchet::PreparedSend,
    expires_at_secs: Option<u64>,
) -> anyhow::Result<Option<PreparedPublishBatch>> {
    let invite_events = prepared
        .invite_responses
        .iter()
        .map(codec::invite_response_event)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let message_events = prepared
        .deliveries
        .iter()
        .map(|delivery| message_event_with_expiration(&delivery.envelope, expires_at_secs))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if message_events.is_empty() {
        return Ok(None);
    }

    Ok(Some(PreparedPublishBatch {
        invite_events,
        message_events,
    }))
}

pub(super) fn build_group_prepared_publish_batch(
    prepared: &nostr_double_ratchet::GroupPreparedSend,
    expires_at_secs: Option<u64>,
) -> anyhow::Result<Option<PreparedPublishBatch>> {
    build_group_publish_batch(&prepared.remote, expires_at_secs)
}

pub(super) fn build_group_local_sibling_publish_batch(
    prepared: &nostr_double_ratchet::GroupPreparedSend,
) -> anyhow::Result<Option<PreparedPublishBatch>> {
    build_group_publish_batch(&prepared.local_sibling, None)
}

pub(super) fn build_group_publish_batch(
    prepared: &nostr_double_ratchet::GroupPreparedPublish,
    expires_at_secs: Option<u64>,
) -> anyhow::Result<Option<PreparedPublishBatch>> {
    let invite_events = prepared
        .invite_responses
        .iter()
        .map(codec::invite_response_event)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let message_events = prepared
        .deliveries
        .iter()
        .map(|delivery| message_event_with_expiration(&delivery.envelope, expires_at_secs))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if message_events.is_empty() {
        return Ok(None);
    }

    Ok(Some(PreparedPublishBatch {
        invite_events,
        message_events,
    }))
}

fn message_event_with_expiration(
    envelope: &MessageEnvelope,
    expires_at_secs: Option<u64>,
) -> anyhow::Result<Event> {
    let Some(expires_at_secs) = expires_at_secs else {
        return Ok(codec::message_event(envelope)?);
    };
    let author_secret_key = SecretKey::from_slice(&envelope.signer_secret_key)?;
    let author_keys = Keys::new(author_secret_key);
    let derived_sender = DevicePubkey::from_bytes(author_keys.public_key().to_bytes());
    if derived_sender != envelope.sender {
        anyhow::bail!("sender does not match signer secret");
    }
    let unsigned = EventBuilder::new(
        Kind::from(codec::MESSAGE_EVENT_KIND as u16),
        envelope.ciphertext.clone(),
    )
    .tag(nostr::Tag::parse([
        "header",
        envelope.encrypted_header.as_str(),
    ])?)
    .tag(nostr::Tag::parse([
        "expiration",
        &expires_at_secs.to_string(),
    ])?)
    .custom_created_at(Timestamp::from(envelope.created_at.get()))
    .build(author_keys.public_key());
    Ok(unsigned.sign_with_keys(&author_keys)?)
}

pub(super) fn publish_mode_for_batch(batch: &PreparedPublishBatch) -> OutboundPublishMode {
    if batch.invite_events.is_empty() {
        OutboundPublishMode::OrdinaryFirstAck
    } else {
        OutboundPublishMode::FirstContactStaged
    }
}

pub(super) fn migrate_publish_mode(
    current: OutboundPublishMode,
    batch: Option<&PreparedPublishBatch>,
) -> OutboundPublishMode {
    match current {
        OutboundPublishMode::WaitForPeer => batch
            .map(publish_mode_for_batch)
            .unwrap_or(OutboundPublishMode::WaitForPeer),
        other => other,
    }
}

pub(super) fn pending_reason_for_publish_mode(mode: &OutboundPublishMode) -> PendingSendReason {
    match mode {
        OutboundPublishMode::FirstContactStaged => PendingSendReason::PublishingFirstContact,
        OutboundPublishMode::OrdinaryFirstAck => PendingSendReason::PublishRetry,
        OutboundPublishMode::WaitForPeer => PendingSendReason::MissingDeviceInvite,
    }
}

pub(super) fn retry_delay_for_publish_mode(mode: &OutboundPublishMode) -> u64 {
    match mode {
        OutboundPublishMode::FirstContactStaged => FIRST_CONTACT_RETRY_DELAY_SECS,
        OutboundPublishMode::OrdinaryFirstAck | OutboundPublishMode::WaitForPeer => {
            PENDING_RETRY_DELAY_SECS
        }
    }
}

pub(super) fn retry_deadline_for_publish_mode(now_secs: u64, mode: &OutboundPublishMode) -> u64 {
    now_secs.saturating_add(retry_delay_for_publish_mode(mode))
}
