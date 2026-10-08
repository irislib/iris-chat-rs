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
            if let Some(value) = from_snapshot(core, message) {
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
            if matches!(
                message.delivery,
                PersistedDeliveryState::Queued | PersistedDeliveryState::Pending
            ) && !super::super::direct_files::is_pending_self_offer(
                &message.body,
                &message.chat_id,
                message.author_owner_pubkey_hex.as_deref(),
                message.is_outgoing,
            ) {
                continue;
            }
            if let Some(value) = from_persisted(core, message) {
                insert_bounded(&mut messages, value, limit);
            }
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
        && match message.delivery {
            DeliveryState::Failed => false,
            DeliveryState::Queued | DeliveryState::Pending => {
                super::super::direct_files::is_pending_self_offer(
                    &message.body,
                    &message.chat_id,
                    message.author_owner_pubkey_hex.as_deref(),
                    message.is_outgoing,
                )
            }
            _ => true,
        }
}

fn after_cursor(message: &ChatMessageSnapshot, cursor: &DeviceSyncCursor) -> bool {
    (message.created_at_secs, &message.chat_id, &message.id)
        > (cursor.created_at, &cursor.chat_id, &cursor.id)
}

pub(super) fn from_snapshot(
    core: &AppCore,
    message: &ChatMessageSnapshot,
) -> Option<DeviceSyncMessage> {
    redact_deleted_body(
        core,
        DeviceSyncMessage {
            legacy_reactions: None,
            chat_id: message.chat_id.clone(),
            id: message.id.clone(),
            body: message
                .edit_history
                .first()
                .map(|v| v.body.clone())
                .unwrap_or_else(|| message_wire_text(&message.body, &message.attachments)),
            author: message.author_owner_pubkey_hex.clone()?,
            created_at: message.created_at_secs,
            expires_at: message.expires_at_secs,
        },
    )
}

fn from_persisted(core: &AppCore, message: PersistedMessage) -> Option<DeviceSyncMessage> {
    let body = message
        .edit_history
        .first()
        .map(|v| v.body.clone())
        .unwrap_or_else(|| message_wire_text(&message.body, &message.attachments));
    redact_deleted_body(
        core,
        DeviceSyncMessage {
            legacy_reactions: None,
            chat_id: message.chat_id,
            id: message.id,
            body,
            author: message.author_owner_pubkey_hex.unwrap_or(message.author),
            created_at: message.created_at_secs,
            expires_at: message.expires_at_secs,
        },
    )
}

fn message_key(message: &DeviceSyncMessage) -> MessageKey {
    (
        message.created_at,
        message.chat_id.clone(),
        message.id.clone(),
    )
}

// A mutation head may be durable while its message-row projection is awaiting
// retry. Never export retained plaintext in that interval, including unloaded rows.
fn redact_deleted_body(
    core: &AppCore,
    mut message: DeviceSyncMessage,
) -> Option<DeviceSyncMessage> {
    if !message.body.is_empty() {
        let records = core
            .message_mutation_records_result(&message.chat_id, &message.id)
            .ok()?;
        if records.iter().any(|record| {
            record.operation == "delete"
                && record.author == message.author
                && record.created_at >= message.created_at
                && record
                    .expires_at
                    .is_none_or(|expiry| expiry > unix_now().get())
        }) {
            message.body.clear();
        }
    }
    Some(message)
}

pub(super) fn history_message_allowed(core: &AppCore, message: &DeviceSyncMessage) -> bool {
    valid_device_sync_chat_id(&message.chat_id)
        && !message.id.is_empty()
        && message.id.len() <= 128
        && message.body.len() <= 32 * 1024
        && PublicKey::from_hex(&message.author).is_ok()
        && core.block_allows_history(&message.chat_id, &message.author, message.created_at)
        && !core.chat_activity_is_deleted(&message.chat_id, message.created_at)
        && !core
            .app_store
            .message_was_locally_deleted(&message.chat_id, Some(&message.id), None)
            .unwrap_or(true)
        && message
            .expires_at
            .is_none_or(|until| until > unix_now().get())
}

pub(super) fn load_history_message(
    core: &AppCore,
    cursor: &DeviceSyncCursor,
) -> Option<DeviceSyncMessage> {
    let message = if let Some(message) = in_memory_message(core, &cursor.chat_id, &cursor.id) {
        if !eligible(message, 0, unix_now().get()) {
            return None;
        }
        from_snapshot(core, message)?
    } else {
        let message = core
            .app_store
            .load_messages_around(&cursor.chat_id, &cursor.id, 0, 0)
            .ok()?
            .into_iter()
            .next()?;
        if !matches!(message.kind, ChatMessageKind::User)
            || matches!(
                message.delivery,
                PersistedDeliveryState::Failed
                    | PersistedDeliveryState::Queued
                    | PersistedDeliveryState::Pending
            )
        {
            return None;
        }
        from_persisted(core, message)?
    };
    (message.created_at == cursor.created_at && history_message_allowed(core, &message))
        .then_some(message)
}
