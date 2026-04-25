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

pub(super) fn event_message_ids(event: &UnsignedEvent) -> Vec<String> {
    event
        .tags
        .iter()
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

pub(super) fn chat_id_for_rumor(
    sender_owner: PublicKey,
    local_owner: PublicKey,
    event: &UnsignedEvent,
) -> String {
    if let Some(group_id) = first_tag_value(event.tags.iter(), "l") {
        return group_chat_id(&group_id);
    }
    if sender_owner == local_owner {
        if let Some(peer_hex) = first_tag_value(event.tags.iter(), "p") {
            if let Ok(peer) = PublicKey::parse(&peer_hex) {
                if peer != local_owner {
                    return peer.to_hex();
                }
            }
        }
    }
    sender_owner.to_hex()
}

pub(super) fn chat_settings_ttl_seconds(content: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    value
        .get("messageTtlSeconds")
        .or_else(|| value.get("message_ttl_seconds"))
        .and_then(serde_json::Value::as_u64)
}

pub(super) fn send_options_for_expiration(expires_at_secs: Option<u64>) -> Option<SendOptions> {
    expires_at_secs.map(|expires_at| SendOptions {
        expires_at: Some(expires_at),
        ttl_seconds: None,
    })
}
