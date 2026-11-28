use super::*;

pub(crate) struct MigratedThreadHistory {
    pub(crate) skipped_loaded_ids: HashSet<String>,
    pub(crate) unread_count: u64,
    pub(crate) updated_at_secs: u64,
    pub(crate) draft: String,
    pub(in crate::core) read_state: Option<ChatReadState>,
}

impl AppStore {
    /// Move an already-authorized device alias in SQLite, including unloaded
    /// history. Only the caller's loaded IDs and the merged header enter memory.
    /// The existing target wins duplicate IDs and duplicate source event IDs.
    pub(crate) fn migrate_direct_thread_history(
        &mut self,
        source: &ThreadRecord,
        target: Option<&ThreadRecord>,
        to: &str,
        from_label: &str,
        to_label: &str,
    ) -> anyhow::Result<MigratedThreadHistory> {
        let from = &source.chat_id;
        anyhow::ensure!(from != to, "alias source equals target");
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        upsert_unsaved_thread_rows(&tx, self.cache.threads.get(from), from, source)?;
        if let Some(target) = target {
            upsert_unsaved_thread_rows(&tx, self.cache.threads.get(to), to, target)?;
        }
        tx.execute("INSERT OR IGNORE INTO threads(chat_id) VALUES (?1)", [to])?;
        let (target_unread, target_updated, draft): (u64, u64, String) = tx.query_row(
            "SELECT unread_count, updated_at_secs, draft FROM threads WHERE chat_id=?1",
            [to],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let mut unread_count = target_unread.saturating_add(source.unread_count);
        let updated_at_secs = target_updated.max(source.updated_at_secs);
        tx.execute(
            "UPDATE threads SET unread_count=?2, updated_at_secs=?3 WHERE chat_id=?1",
            params![to, unread_count as i64, updated_at_secs as i64],
        )?;

        // Set membership uses SQLite's indexed/materialized subqueries rather
        // than one history-sized Rust collection or an all-pairs message scan.
        tx.execute(
            "DELETE FROM messages WHERE chat_id=?1 AND (
            id IN (SELECT id FROM messages WHERE chat_id=?2)
            OR source_event_id IN (SELECT source_event_id FROM messages
                WHERE chat_id=?2 AND source_event_id IS NOT NULL)
            OR EXISTS (SELECT 1 FROM app_meta WHERE key IN (
                ?3 || 'id:' || messages.id, ?3 || 'source:' || messages.source_event_id)))",
            params![from, to, format!("message_deleted:{}:{to}:", to.len())],
        )?;
        let loaded_ids = source
            .messages
            .iter()
            .map(|message| &message.id)
            .collect::<Vec<_>>();
        let skipped_loaded_ids = {
            let mut query = tx.prepare(
                "SELECT value FROM json_each(?2) WHERE NOT EXISTS (
                SELECT 1 FROM messages WHERE chat_id=?1 AND id=json_each.value)",
            )?;
            let rows = query
                .query_map(params![from, serde_json::to_string(&loaded_ids)?], |row| {
                    row.get::<_, String>(0)
                })?;
            rows.collect::<Result<HashSet<_>, _>>()?
        };
        // Update only local projection fields. Signed events, original author
        // ownership, attachments, edits and outgoing retry bytes stay intact.
        tx.execute(
            "UPDATE messages SET chat_id=?2,
            author=CASE WHEN is_outgoing=0 AND author IN (?1,?3) THEN ?4 ELSE author END,
            recipient_deliveries_json=(SELECT json_group_array(json(CASE
                WHEN json_extract(value,'$.owner_pubkey_hex')=?1
                    THEN json_set(value,'$.owner_pubkey_hex',?2) ELSE value END))
                FROM json_each(messages.recipient_deliveries_json)),
            reactors_json=(SELECT json_group_array(json(CASE
                WHEN json_extract(value,'$.author')=?1
                    THEN json_set(value,'$.author',?2) ELSE value END))
                FROM json_each(messages.reactors_json))
            WHERE chat_id=?1",
            params![from, to, from_label, to_label],
        )?;
        tx.execute("INSERT OR IGNORE INTO pending_message_reads(chat_id,id,created_at_secs,expires_at_secs)
            SELECT ?2,id,created_at_secs,expires_at_secs FROM pending_message_reads WHERE chat_id=?1",
            params![from, to])?;
        tx.execute("DELETE FROM pending_message_reads WHERE chat_id=?1", [from])?;
        let read_state = store_chat_read_states::migrate_alias_read_state(&tx, from, to)?;
        if read_state.is_some() {
            unread_count = tx.query_row(
                "SELECT unread_count FROM threads WHERE chat_id=?1",
                [to],
                |row| row.get(0),
            )?;
        }
        tx.execute(
            "UPDATE messages SET delivery='seen' WHERE chat_id=?1 AND is_outgoing=0
            AND delivery!='seen' AND id IN (SELECT id FROM pending_message_reads WHERE chat_id=?1)
            AND EXISTS (SELECT 1 FROM pending_message_reads AS pending WHERE pending.chat_id=?1
                AND pending.id=messages.id AND pending.created_at_secs=messages.created_at_secs)",
            [to],
        )?;
        tx.execute(
            "DELETE FROM pending_message_reads WHERE chat_id=?1
            AND id IN (SELECT id FROM messages WHERE chat_id=?1)",
            [to],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO chat_message_ttls(chat_id,ttl_seconds)
            SELECT ?2,ttl_seconds FROM chat_message_ttls WHERE chat_id=?1",
            params![from, to],
        )?;
        tx.execute("DELETE FROM chat_message_ttls WHERE chat_id=?1", [from])?;
        tx.execute(
            "UPDATE app_meta SET value=?2 WHERE key=?3 AND value=?1",
            params![from, to, META_ACTIVE_CHAT_ID],
        )?;
        // No source rows remain to cascade. FTS tracks each moved row through
        // its existing UPDATE trigger, retaining the same rowid and body.
        tx.execute("DELETE FROM threads WHERE chat_id=?1", [from])?;
        tx.commit()?;

        let owned_source = self
            .cache
            .threads
            .remove(from)
            .is_some_and(|entry| entry.in_snapshot);
        let entry = self.cache.threads.entry(to.to_string()).or_default();
        entry.in_snapshot |= owned_source;
        entry.header_hash = None;
        entry.messages.clear();
        self.cache.meta = None;
        self.cache.chat_ttls = None;
        Ok(MigratedThreadHistory {
            skipped_loaded_ids,
            unread_count,
            updated_at_secs,
            draft,
            read_state,
        })
    }
}
