use super::*;
use std::collections::BTreeMap;

type MessageKey = (u64, String, String);

pub(super) fn collect_device_sync_messages(
    core: &AppCore,
    roster_at: u64,
    after: Option<&DeviceSyncCursor>,
    page_size: usize,
) -> (Vec<DeviceSyncMessage>, Option<DeviceSyncCursor>) {
    let now = unix_now().get();
    let limit = page_size.saturating_add(1);
    let mut messages = BTreeMap::new();
    for message in core.threads.values().flat_map(|thread| &thread.messages) {
        if eligible(message, roster_at, now)
            && after.is_none_or(|cursor| after_cursor(message, cursor))
        {
            if let Some(value) = from_snapshot(message) {
                insert_bounded(&mut messages, value, limit);
            }
        }
    }

    let mut db_after = after
        .map(|cursor| (cursor.created_at, cursor.chat_id.clone(), cursor.id.clone()))
        .unwrap_or((roster_at, String::new(), String::new()));
    loop {
        let rows = core
            .app_store
            .load_device_sync_messages_page(
                roster_at,
                now,
                db_after.0,
                &db_after.1,
                &db_after.2,
                limit,
            )
            .unwrap_or_default();
        let exhausted = rows.len() < limit;
        let Some(last) = rows.last() else { break };
        let frontier = (last.created_at_secs, last.chat_id.clone(), last.id.clone());
        for message in rows {
            if in_memory_message(core, &message.chat_id, &message.id).is_some() {
                continue;
            }
            insert_bounded(&mut messages, from_persisted(message), limit);
        }
        let page_is_known = messages
            .last_key_value()
            .is_some_and(|(last, _)| messages.len() >= limit && *last <= frontier);
        db_after = frontier;
        if exhausted || page_is_known {
            break;
        }
    }

    let has_more = messages.len() > page_size;
    while messages.len() > page_size {
        messages.pop_last();
    }
    let values = messages.into_values().collect::<Vec<_>>();
    let next = has_more
        .then(|| values.last().map(DeviceSyncCursor::from))
        .flatten();
    (values, next)
}

fn insert_bounded(
    messages: &mut BTreeMap<MessageKey, DeviceSyncMessage>,
    value: DeviceSyncMessage,
    limit: usize,
) {
    messages.insert(message_key(&value), value);
    while messages.len() > limit {
        messages.pop_last();
    }
}

fn in_memory_message<'a>(
    core: &'a AppCore,
    chat_id: &str,
    id: &str,
) -> Option<&'a ChatMessageSnapshot> {
    core.threads
        .get(chat_id)?
        .messages
        .iter()
        .find(|message| message.id == id)
}

fn eligible(message: &ChatMessageSnapshot, roster_at: u64, now: u64) -> bool {
    message.created_at_secs >= roster_at
        && message
            .expires_at_secs
            .is_none_or(|expires_at| expires_at > now)
        && matches!(message.kind, ChatMessageKind::User)
        && !matches!(
            message.delivery,
            DeliveryState::Queued | DeliveryState::Pending | DeliveryState::Failed
        )
}

fn after_cursor(message: &ChatMessageSnapshot, cursor: &DeviceSyncCursor) -> bool {
    (message.created_at_secs, &message.chat_id, &message.id)
        > (cursor.created_at, &cursor.chat_id, &cursor.id)
}

fn from_snapshot(message: &ChatMessageSnapshot) -> Option<DeviceSyncMessage> {
    Some(DeviceSyncMessage {
        chat_id: message.chat_id.clone(),
        id: message.id.clone(),
        body: message.body.clone(),
        author: message.author_owner_pubkey_hex.clone()?,
        created_at: message.created_at_secs,
        expires_at: message.expires_at_secs,
    })
}

fn from_persisted(message: PersistedMessage) -> DeviceSyncMessage {
    DeviceSyncMessage {
        chat_id: message.chat_id,
        id: message.id,
        body: message.body,
        author: message.author_owner_pubkey_hex.unwrap_or(message.author),
        created_at: message.created_at_secs,
        expires_at: message.expires_at_secs,
    }
}

fn message_key(message: &DeviceSyncMessage) -> MessageKey {
    (
        message.created_at,
        message.chat_id.clone(),
        message.id.clone(),
    )
}
