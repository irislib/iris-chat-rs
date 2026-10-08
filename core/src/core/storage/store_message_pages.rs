use super::*;

pub(crate) fn load_recent_messages(
    conn: &rusqlite::Connection,
    chat_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<PersistedMessage>> {
    load_recent_messages_with_visibility(conn, chat_id, limit, None, None)
}

pub(crate) fn load_recent_messages_with_visibility(
    conn: &rusqlite::Connection,
    chat_id: &str,
    limit: usize,
    hidden_authors_json: Option<&str>,
    blocked_intervals_json: Option<&str>,
) -> anyhow::Result<Vec<PersistedMessage>> {
    let mut stmt = conn.prepare(
        "SELECT chat_id, id, kind, author, author_owner_pubkey_hex, body, is_outgoing, created_at_secs, expires_at_secs,
	                delivery, attachments_json, reactions_json, reactors_json, source_event_id,
	                recipient_deliveries_json, delivery_trace_json, call_json, system_notice_owner_pubkey_hex, edit_history_json, deleted_for_everyone
	         FROM (
	             SELECT chat_id, id, kind, author, author_owner_pubkey_hex, body, is_outgoing, created_at_secs, expires_at_secs,
	                    delivery, attachments_json, reactions_json, reactors_json, source_event_id,
	                    recipient_deliveries_json, delivery_trace_json, call_json, system_notice_owner_pubkey_hex, edit_history_json, deleted_for_everyone,
	                    rowid AS storage_order
	             FROM messages
	             WHERE chat_id = ?1
                     AND (?3 IS NULL OR kind != 'user' OR is_outgoing != 0 OR author_owner_pubkey_hex IS NULL
                          OR author_owner_pubkey_hex NOT IN (SELECT value FROM json_each(?3)))
                   AND (kind != 'user' OR is_outgoing != 0 OR NOT EXISTS (
                       SELECT 1 FROM json_each(?4) AS blocked
                       WHERE json_extract(blocked.value, '$.author') = author_owner_pubkey_hex
                         AND created_at_secs >= json_extract(blocked.value, '$.since')
                         AND (json_extract(blocked.value, '$.until') IS NULL
                              OR created_at_secs < json_extract(blocked.value, '$.until'))))
	             ORDER BY created_at_secs DESC, storage_order DESC
	             LIMIT ?2
	         )
	         ORDER BY created_at_secs ASC, storage_order ASC",
    )?;
    let rows = stmt.query_map(
        params![
            chat_id,
            limit as i64,
            hidden_authors_json,
            blocked_intervals_json
        ],
        persisted_message_from_row,
    )?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    Ok(messages)
}

#[cfg(test)]
pub(crate) fn load_messages_before(
    conn: &rusqlite::Connection,
    chat_id: &str,
    before_message_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<PersistedMessage>> {
    load_messages_before_with_visibility(conn, chat_id, before_message_id, limit, None, None)
}

pub(crate) fn load_messages_before_with_visibility(
    conn: &rusqlite::Connection,
    chat_id: &str,
    before_message_id: &str,
    limit: usize,
    hidden_authors_json: Option<&str>,
    blocked_intervals_json: Option<&str>,
) -> anyhow::Result<Vec<PersistedMessage>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "WITH anchor AS (
	             SELECT created_at_secs AS anchor_created,
	                    rowid AS anchor_storage_order
	             FROM messages
	             WHERE chat_id = ?1 AND id = ?2
	             LIMIT 1
	         )
         SELECT chat_id, id, kind, author, author_owner_pubkey_hex, body, is_outgoing, created_at_secs, expires_at_secs,
                delivery, attachments_json, reactions_json, reactors_json, source_event_id,
                recipient_deliveries_json, delivery_trace_json, call_json, system_notice_owner_pubkey_hex, edit_history_json, deleted_for_everyone
         FROM (
             SELECT m.chat_id, m.id, m.kind, m.author, m.author_owner_pubkey_hex, m.body, m.is_outgoing,
	                    m.created_at_secs, m.expires_at_secs, m.delivery, m.attachments_json,
	                    m.reactions_json, m.reactors_json, m.source_event_id,
	                    m.recipient_deliveries_json, m.delivery_trace_json, m.call_json, m.system_notice_owner_pubkey_hex, m.edit_history_json, m.deleted_for_everyone,
	                    m.rowid AS storage_order
	             FROM messages m, anchor
	             WHERE m.chat_id = ?1
                   AND (?4 IS NULL OR m.kind != 'user' OR m.is_outgoing != 0 OR m.author_owner_pubkey_hex IS NULL
                        OR m.author_owner_pubkey_hex NOT IN (SELECT value FROM json_each(?4)))
                   AND (m.kind != 'user' OR m.is_outgoing != 0 OR NOT EXISTS (
                       SELECT 1 FROM json_each(?5) AS blocked
                       WHERE json_extract(blocked.value, '$.author') = m.author_owner_pubkey_hex
                         AND m.created_at_secs >= json_extract(blocked.value, '$.since')
                         AND (json_extract(blocked.value, '$.until') IS NULL
                              OR m.created_at_secs < json_extract(blocked.value, '$.until'))))
	               AND (
	                    m.created_at_secs < anchor.anchor_created
	                    OR (
	                        m.created_at_secs = anchor.anchor_created
	                        AND m.rowid < anchor.anchor_storage_order
	                    )
	               )
	             ORDER BY m.created_at_secs DESC, storage_order DESC
	             LIMIT ?3
	         )
	         ORDER BY created_at_secs ASC, storage_order ASC",
    )?;
    let rows = stmt.query_map(
        params![
            chat_id,
            before_message_id,
            limit as i64,
            hidden_authors_json,
            blocked_intervals_json
        ],
        persisted_message_from_row,
    )?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    Ok(messages)
}

pub(crate) fn load_messages_around(
    conn: &rusqlite::Connection,
    chat_id: &str,
    message_id: &str,
    before_limit: usize,
    after_limit: usize,
) -> anyhow::Result<Vec<PersistedMessage>> {
    load_messages_around_with_visibility(
        conn,
        chat_id,
        message_id,
        before_limit,
        after_limit,
        None,
        None,
    )
}

pub(crate) fn load_messages_around_with_visibility(
    conn: &rusqlite::Connection,
    chat_id: &str,
    message_id: &str,
    before_limit: usize,
    after_limit: usize,
    hidden_authors_json: Option<&str>,
    blocked_intervals_json: Option<&str>,
) -> anyhow::Result<Vec<PersistedMessage>> {
    let before = load_messages_before_with_visibility(
        conn,
        chat_id,
        message_id,
        before_limit,
        hidden_authors_json,
        blocked_intervals_json,
    )?;
    let mut stmt = conn.prepare(
        "WITH anchor AS (
	             SELECT created_at_secs AS anchor_created,
	                    rowid AS anchor_storage_order
	             FROM messages
	             WHERE chat_id = ?1 AND id = ?2
	             LIMIT 1
	         )
         SELECT m.chat_id, m.id, m.kind, m.author, m.author_owner_pubkey_hex, m.body, m.is_outgoing, m.created_at_secs,
                m.expires_at_secs, m.delivery, m.attachments_json, m.reactions_json,
                m.reactors_json, m.source_event_id, m.recipient_deliveries_json,
                m.delivery_trace_json, m.call_json, m.system_notice_owner_pubkey_hex, m.edit_history_json, m.deleted_for_everyone
         FROM messages m, anchor
         WHERE m.chat_id = ?1
                   AND (?4 IS NULL OR m.kind != 'user' OR m.is_outgoing != 0 OR m.author_owner_pubkey_hex IS NULL
                        OR m.author_owner_pubkey_hex NOT IN (SELECT value FROM json_each(?4)))
                   AND (m.kind != 'user' OR m.is_outgoing != 0 OR NOT EXISTS (
                       SELECT 1 FROM json_each(?5) AS blocked
                       WHERE json_extract(blocked.value, '$.author') = m.author_owner_pubkey_hex
                         AND m.created_at_secs >= json_extract(blocked.value, '$.since')
                         AND (json_extract(blocked.value, '$.until') IS NULL
                              OR m.created_at_secs < json_extract(blocked.value, '$.until'))))
           AND (
	                m.created_at_secs > anchor.anchor_created
	                OR (
	                    m.created_at_secs = anchor.anchor_created
	                    AND m.rowid >= anchor.anchor_storage_order
	                )
	           )
	         ORDER BY m.created_at_secs ASC,
	                  m.rowid ASC
	         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            chat_id,
            message_id,
            after_limit.saturating_add(1) as i64,
            hidden_authors_json,
            blocked_intervals_json
        ],
        persisted_message_from_row,
    )?;
    let mut messages = before;
    for row in rows {
        messages.push(row?);
    }
    Ok(messages)
}
