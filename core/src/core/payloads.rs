use super::*;

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

const APP_GROUP_MESSAGE_PAYLOAD_VERSION: u8 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AppGroupMessagePayload {
    pub(super) version: u8,
    pub(super) body: String,
    pub(super) message_id: String,
}

pub(super) fn encode_app_group_message_payload(
    body: &str,
    message_id: &str,
) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AppGroupMessagePayload {
        version: APP_GROUP_MESSAGE_PAYLOAD_VERSION,
        body: body.to_string(),
        message_id: message_id.to_string(),
    })?)
}

pub(super) fn decode_app_group_message_payload(payload: &[u8]) -> Option<AppGroupMessagePayload> {
    let decoded = serde_json::from_slice::<AppGroupMessagePayload>(payload).ok()?;
    (decoded.version == APP_GROUP_MESSAGE_PAYLOAD_VERSION && !decoded.message_id.is_empty())
        .then_some(decoded)
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

pub(super) fn first_tag_value<'a>(
    tags: impl IntoIterator<Item = &'a nostr::Tag>,
    name: &str,
) -> Option<String> {
    tags.into_iter()
        .find(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some(name))
        .and_then(|tag| tag.as_slice().get(1).cloned())
}

pub(super) fn message_ids_from_tags<'a>(
    tags: impl IntoIterator<Item = &'a nostr::Tag>,
) -> Vec<String> {
    tags.into_iter()
        .filter(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some("e"))
        .filter_map(|tag| tag.as_slice().get(1).cloned())
        .collect()
}

pub(super) fn message_expiration_from_tags<'a>(
    tags: impl IntoIterator<Item = &'a nostr::Tag>,
) -> Option<u64> {
    let raw = tags
        .into_iter()
        .find(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some("expiration"))
        .and_then(|tag| tag.as_slice().get(1))?;
    let mut value = raw.parse::<u64>().ok()?;
    if value == 0 {
        return None;
    }
    while value > 9_999_999_999 {
        value /= 1_000;
    }
    (value > 0).then_some(value)
}

pub(super) fn chat_id_for_tags<'a>(
    sender_owner: PublicKey,
    local_owner: PublicKey,
    tags: impl IntoIterator<Item = &'a nostr::Tag>,
) -> String {
    let tags = tags.into_iter().collect::<Vec<_>>();
    if let Some(group_id) = first_tag_value(tags.iter().copied(), "l") {
        return group_chat_id(&group_id);
    }
    if sender_owner == local_owner {
        if let Some(peer_hex) = first_tag_value(tags.iter().copied(), "p") {
            if let Ok(peer) = PublicKey::parse(&peer_hex) {
                if peer != local_owner {
                    return peer.to_hex();
                }
            }
        }
    }
    sender_owner.to_hex()
}

pub(super) fn chat_id_for_runtime_message<'a>(
    sender_owner: PublicKey,
    local_owner: PublicKey,
    conversation_owner: Option<PublicKey>,
    tags: impl IntoIterator<Item = &'a nostr::Tag>,
) -> String {
    let chat_id = chat_id_for_tags(sender_owner, local_owner, tags);
    if is_group_chat_id(&chat_id) {
        return chat_id;
    }
    direct_self_sync_chat_id(sender_owner, local_owner, conversation_owner).unwrap_or(chat_id)
}

pub(super) fn direct_self_sync_chat_id(
    sender_owner: PublicKey,
    local_owner: PublicKey,
    conversation_owner: Option<PublicKey>,
) -> Option<String> {
    let owner = conversation_owner?;
    if sender_owner == local_owner && owner != local_owner {
        Some(owner.to_hex())
    } else {
        None
    }
}

pub(super) struct RuntimeRumor {
    pub(super) id: Option<String>,
    pub(super) kind: u32,
    pub(super) content: String,
    pub(super) created_at_secs: u64,
    pub(super) tags: Vec<nostr::Tag>,
}

#[derive(Deserialize)]
struct LooseRuntimeRumor {
    #[serde(default)]
    id: Option<String>,
    kind: u32,
    content: String,
    created_at: u64,
    #[serde(default)]
    tags: Vec<Vec<String>>,
}

pub(super) fn parse_runtime_rumor(content: &str) -> Option<RuntimeRumor> {
    if let Ok(event) = serde_json::from_str::<UnsignedEvent>(content) {
        return Some(RuntimeRumor {
            id: event.id.as_ref().map(ToString::to_string),
            kind: event.kind.as_u16() as u32,
            content: event.content.clone(),
            created_at_secs: event.created_at.as_secs(),
            tags: event.tags.iter().cloned().collect(),
        });
    }

    let loose = serde_json::from_str::<LooseRuntimeRumor>(content).ok()?;
    let tags = loose
        .tags
        .iter()
        .filter_map(|tag| nostr::Tag::parse(tag.iter().map(String::as_str)).ok())
        .collect();
    Some(RuntimeRumor {
        id: loose.id,
        kind: loose.kind,
        content: loose.content,
        created_at_secs: loose.created_at,
        tags,
    })
}

pub(super) fn chat_settings_ttl_seconds(content: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    if let Some(ttl) = value.as_u64() {
        return Some(ttl);
    }
    value
        .get("messageTtlSeconds")
        .or_else(|| value.get("message_ttl_seconds"))
        .and_then(serde_json::Value::as_u64)
}
