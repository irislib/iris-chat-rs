// Group notices use device-local IDs. An outgoing reply gives each device
// a common point at which it can acknowledge its own older notices.
use rusqlite::{params, Transaction};

pub(super) fn has_unread_notice(
    tx: &Transaction<'_>,
    chat_id: &str,
    through: i64,
) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM messages WHERE chat_id = ?1 AND kind = 'system'
         AND is_outgoing = 0 AND delivery != 'seen' AND created_at_secs <= ?2)",
        params![chat_id, through],
        |row| row.get(0),
    )
}

pub(super) fn acknowledge(
    tx: &Transaction<'_>,
    chat_id: &str,
    through: i64,
    unread_count: i64,
) -> anyhow::Result<i64> {
    // Only the latest unread_count incoming rows can own the badge. A notice
    // still marked Received outside that window must not clear a peer's badge.
    let counted: i64 = tx.query_row(
        "SELECT count(*) FROM (
            SELECT kind, created_at_secs FROM messages
            WHERE chat_id = ?1 AND is_outgoing = 0 AND delivery != 'seen'
            ORDER BY created_at_secs DESC, rowid DESC LIMIT ?2
         ) WHERE kind = 'system' AND created_at_secs <= ?3",
        params![chat_id, unread_count, through],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE messages SET delivery = 'seen' WHERE chat_id = ?1 AND kind = 'system'
         AND is_outgoing = 0 AND delivery != 'seen' AND created_at_secs <= ?2",
        params![chat_id, through],
    )?;
    let remaining = unread_count - counted;
    if counted > 0 {
        tx.execute(
            "UPDATE threads SET unread_count = ?2 WHERE chat_id = ?1",
            params![chat_id, remaining],
        )?;
    }
    Ok(remaining)
}

pub(super) fn migrate(tx: &Transaction<'_>) -> anyhow::Result<()> {
    let mut chats = tx.prepare(
        "SELECT chat_id, unread_count, (SELECT max(created_at_secs) FROM messages
            WHERE chat_id = threads.chat_id AND kind = 'user' AND is_outgoing = 1)
         FROM threads WHERE chat_id GLOB 'group:*' AND unread_count > 0
         AND EXISTS (SELECT 1 FROM messages WHERE chat_id = threads.chat_id
            AND kind = 'system' AND is_outgoing = 0 AND delivery != 'seen')",
    )?;
    let rows = chats
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (chat, unread, through) in rows {
        if let Some(through) = through {
            acknowledge(tx, &chat, through, unread)?;
        }
    }
    Ok(())
}
