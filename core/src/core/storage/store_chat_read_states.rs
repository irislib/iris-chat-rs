use super::*;
use crate::core::{unix_now, MessageReadState};

const READ_STATE_PREFIX: &str = "chat_read_state:";

impl AppStore {
    pub(crate) fn load_chat_read_states(&self) -> anyhow::Result<BTreeMap<String, ChatReadState>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut statement =
            conn.prepare("SELECT key, value FROM app_meta WHERE key LIKE 'chat_read_state:%'")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut states = BTreeMap::new();
        for row in rows {
            let (key, value) = row?;
            if let Some(chat_id) = key.strip_prefix(READ_STATE_PREFIX) {
                let mut state: ChatReadState = serde_json::from_str(&value)?;
                state.seen_messages = state.exact_messages();
                state.seen_through_secs = 0;
                state.seen_at_boundary.clear();
                state.bound_recent_batch();
                states.insert(chat_id.to_string(), state);
            }
        }
        Ok(states)
    }

    #[cfg(test)]
    pub(crate) fn save_chat_read_state(
        &mut self,
        chat_id: &str,
        state: &ChatReadState,
        thread: Option<&ThreadRecord>,
    ) -> anyhow::Result<Option<u64>> {
        self.save_chat_read_state_batch(chat_id, state, &state.exact_messages(), thread)
    }

    pub(crate) fn save_chat_read_state_batch(
        &mut self,
        chat_id: &str,
        state: &ChatReadState,
        reads: &BTreeMap<String, MessageReadState>,
        thread: Option<&ThreadRecord>,
    ) -> anyhow::Result<Option<u64>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        if let Some(thread) = thread {
            tx.execute(
                "INSERT INTO threads(chat_id, unread_count, updated_at_secs, draft)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(chat_id) DO UPDATE SET
                    unread_count = excluded.unread_count,
                    updated_at_secs = excluded.updated_at_secs,
                    draft = excluded.draft",
                params![
                    chat_id,
                    thread.unread_count as i64,
                    thread.updated_at_secs as i64,
                    thread.draft
                ],
            )?;
            for message in &thread.messages {
                // The row upsert preserves a previously stored incoming Seen.
                upsert_message_row(&tx, chat_id, message)?;
            }
        }

        let prior_unread: Option<i64> = tx
            .query_row(
                "SELECT unread_count FROM threads WHERE chat_id = ?1",
                [chat_id],
                |row| row.get(0),
            )
            .optional()?;
        // Only the most recent counted messages belong to the existing badge.
        // Older Received rows may already have had their badges cleared.
        let remaining_unread = prior_unread
            .map(|prior| -> anyhow::Result<u64> {
                let mut statement = tx.prepare(
                    "SELECT created_at_secs, id FROM messages
                     WHERE chat_id = ?1 AND is_outgoing = 0 AND delivery != 'seen'
                     ORDER BY created_at_secs DESC, rowid DESC LIMIT ?2",
                )?;
                let rows = statement.query_map(params![chat_id, prior.max(0)], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut remaining = prior.max(0) as u64;
                for row in rows {
                    let (created_at, id) = row?;
                    if reads
                        .get(&id)
                        .is_some_and(|read| read.created_at_secs == created_at.max(0) as u64)
                    {
                        remaining = remaining.saturating_sub(1);
                    }
                }
                Ok(remaining)
            })
            .transpose()?;
        tx.execute(
            "INSERT INTO app_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![
                format!("{READ_STATE_PREFIX}{chat_id}"),
                serde_json::to_string(state)?
            ],
        )?;
        for (id, read) in reads {
            if store_message_deletions::contains(&tx, chat_id, Some(id), None)?
                || store_chat_deletions::message_was_deleted(&tx, chat_id, read.created_at_secs)?
                || read
                    .expires_at_secs
                    .is_some_and(|expiry| expiry <= unix_now().get())
            {
                continue;
            }
            tx.execute(
                "UPDATE messages SET delivery = 'seen' WHERE chat_id = ?1 AND id = ?2
                 AND is_outgoing = 0 AND created_at_secs = ?3",
                params![chat_id, id, read.created_at_secs as i64],
            )?;
            // No event log: known messages retain only their existing delivery
            // flag. Unknown originals need one deduplicated pending marker.
            tx.execute(
                "INSERT INTO pending_message_reads(chat_id, id, created_at_secs, expires_at_secs)
                 SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS (
                    SELECT 1 FROM messages WHERE chat_id = ?1 AND id = ?2)
                 ON CONFLICT(chat_id, id) DO UPDATE SET
                    expires_at_secs = COALESCE(pending_message_reads.expires_at_secs, excluded.expires_at_secs)",
                params![chat_id, id, read.created_at_secs as i64, read.expires_at_secs.map(|v| v as i64)],
            )?;
        }
        if let Some(unread_count) = remaining_unread {
            tx.execute(
                "UPDATE threads SET unread_count = ?2 WHERE chat_id = ?1",
                params![chat_id, unread_count as i64],
            )?;
        }
        tx.commit()?;
        self.cache.threads.remove(chat_id);
        Ok(remaining_unread)
    }

    pub(crate) fn message_read_state(
        &self,
        chat_id: &str,
        id: &str,
    ) -> anyhow::Result<Option<MessageReadState>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.query_row(
            "SELECT created_at_secs, expires_at_secs FROM messages
             WHERE chat_id = ?1 AND id = ?2 AND is_outgoing = 0 AND delivery = 'seen'
             UNION ALL SELECT created_at_secs, expires_at_secs FROM pending_message_reads
             WHERE chat_id = ?1 AND id = ?2 LIMIT 1",
            params![chat_id, id],
            |row| {
                Ok(MessageReadState {
                    created_at_secs: row.get::<_, i64>(0)?.max(0) as u64,
                    expires_at_secs: row.get::<_, Option<i64>>(1)?.map(|v| v.max(0) as u64),
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub(crate) fn message_read_page(
        &self,
        after: Option<(&str, &str)>,
    ) -> anyhow::Result<Vec<(String, String, MessageReadState)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let (chat, id) = after.unwrap_or(("", ""));
        let mut query = conn.prepare(
            "SELECT chat_id, id, created_at_secs, expires_at_secs FROM messages
             WHERE is_outgoing = 0 AND delivery = 'seen' AND kind = 'user' AND (chat_id, id) > (?1, ?2)
             UNION ALL SELECT chat_id, id, created_at_secs, expires_at_secs FROM pending_message_reads
             WHERE (chat_id, id) > (?1, ?2)
             ORDER BY chat_id, id LIMIT 256")?;
        let rows = query.query_map(params![chat, id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                MessageReadState {
                    created_at_secs: row.get::<_, i64>(2)?.max(0) as u64,
                    expires_at_secs: row.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as u64),
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn incoming_message_ids(&self, chat_id: &str) -> anyhow::Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut query =
            conn.prepare("SELECT id FROM messages WHERE chat_id = ?1 AND is_outgoing = 0")?;
        let rows = query.query_map([chat_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn remove_chat_read_state(&mut self, chat_id: &str) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.execute(
            "DELETE FROM app_meta WHERE key = ?1",
            [format!("{READ_STATE_PREFIX}{chat_id}")],
        )?;
        Ok(())
    }

    pub(crate) fn migrate_pending_message_reads(
        &mut self,
        from: &str,
        to: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO pending_message_reads(chat_id, id, created_at_secs, expires_at_secs)
            SELECT ?2, id, created_at_secs, expires_at_secs FROM pending_message_reads WHERE chat_id = ?1",
            params![from, to])?;
        tx.execute(
            "DELETE FROM pending_message_reads WHERE chat_id = ?1",
            [from],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::open_database;

    fn sample_read_state() -> ChatReadState {
        ChatReadState {
            updated_at_ms: 12_345,
            device_id: "device-a".to_string(),
            seen_messages: [(
                "boundary-seen".to_string(),
                MessageReadState {
                    created_at_secs: 100,
                    expires_at_secs: None,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }
    }

    fn sample_message(id: &str, created_at_secs: u64, is_outgoing: bool) -> ChatMessageSnapshot {
        ChatMessageSnapshot {
            edit_history: Vec::new(),
            deleted_for_everyone: false,
            system_notice_owner_pubkey_hex: None,
            direct_transfer: None,
            call: None,
            id: id.to_string(),
            chat_id: "chat".to_string(),
            kind: ChatMessageKind::User,
            author: "alice".to_string(),
            author_owner_pubkey_hex: None,
            author_picture_url: None,
            body: "message".to_string(),
            attachments: Vec::new(),
            reactions: Vec::new(),
            reactors: Vec::new(),
            is_outgoing,
            created_at_secs,
            expires_at_secs: None,
            delivery: if is_outgoing {
                DeliveryState::Sent
            } else {
                DeliveryState::Received
            },
            recipient_deliveries: Vec::new(),
            delivery_trace: Default::default(),
            source_event_id: None,
        }
    }

    #[test]
    fn chat_read_states_roundtrip_and_replace_one_chat() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_database(tmp.path()).unwrap();
        let mut store = AppStore::new(conn);
        assert!(store.load_chat_read_states().unwrap().is_empty());
        let first = sample_read_state();
        let mut second = first.clone();
        second.device_id = "device-b".to_string();
        assert_eq!(
            store.save_chat_read_state("chat-a", &first, None).unwrap(),
            None
        );
        store.save_chat_read_state("chat-b", &second, None).unwrap();
        let mut updated = first;
        updated.updated_at_ms += 1;
        store
            .save_chat_read_state("chat-a", &updated, None)
            .unwrap();
        drop(store);

        let mut store = AppStore::new(open_database(tmp.path()).unwrap());
        let states = store.load_chat_read_states().unwrap();
        assert_eq!(states.len(), 2);
        assert!(states.get("chat-a") == Some(&updated));
        assert!(states.get("chat-b") == Some(&second));
        store.remove_chat_read_state("chat-a").unwrap();
        let states = store.load_chat_read_states().unwrap();
        assert_eq!(states.len(), 1);
        assert!(states.get("chat-b") == Some(&second));
    }

    #[test]
    fn chat_read_state_marks_only_covered_incoming_history_seen() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = AppStore::new(open_database(tmp.path()).unwrap());
        {
            let conn = store.conn.lock().unwrap();
            for chat_id in ["chat", "other-chat"] {
                conn.execute(
                    "INSERT INTO threads(chat_id, unread_count) VALUES (?1, 5)",
                    [chat_id],
                )
                .unwrap();
            }
            for (chat_id, id, kind, outgoing, created, delivery) in [
                ("chat", "older", "user", 0, 99, "received"),
                ("chat", "boundary-seen", "user", 0, 100, "received"),
                ("chat", "boundary-unseen", "user", 0, 100, "received"),
                ("chat", "newer", "user", 0, 101, "received"),
                ("chat", "outgoing", "user", 1, 99, "sent"),
                ("chat", "system", "system", 0, 99, "received"),
                ("chat", "already-seen", "user", 0, 101, "seen"),
                ("other-chat", "other", "user", 0, 99, "received"),
            ] {
                conn.execute(
                    "INSERT INTO messages(chat_id, id, kind, author, body,
                        is_outgoing, created_at_secs, delivery)
                     VALUES (?1, ?2, ?3, 'alice', 'message', ?4, ?5, ?6)",
                    params![chat_id, id, kind, outgoing, created, delivery],
                )
                .unwrap();
            }
        }
        store.cache.threads.insert("chat".to_string(), 123);
        store
            .save_chat_read_state("chat", &sample_read_state(), None)
            .unwrap();
        assert!(!store.cache.threads.contains_key("chat"));

        let thread = store.load_thread("chat", 100).unwrap().unwrap();
        assert_eq!(thread.unread_count, 4);
        for message in thread.messages {
            let expected = match message.id.as_str() {
                "boundary-seen" | "already-seen" => "seen",
                "outgoing" => "sent",
                _ => "received",
            };
            let actual = serialize_delivery(&message.delivery.into());
            assert_eq!(actual, expected, "message {}", message.id);
        }
        let other = store.load_thread("other-chat", 100).unwrap().unwrap();
        assert!(matches!(
            other.messages[0].delivery,
            PersistedDeliveryState::Received
        ));

        let mut unread = sample_read_state();
        unread.seen_messages.clear();
        store.save_chat_read_state("chat", &unread, None).unwrap();
        let thread = store.load_thread("chat", 100).unwrap().unwrap();
        assert!(thread
            .messages
            .iter()
            .filter(|message| matches!(message.id.as_str(), "boundary-seen" | "already-seen"))
            .all(|message| matches!(message.delivery, PersistedDeliveryState::Seen)));
        assert!(thread
            .messages
            .iter()
            .find(|message| message.id == "newer")
            .is_some_and(|message| matches!(message.delivery, PersistedDeliveryState::Received)));
    }

    #[test]
    fn chat_read_state_preserves_unread_history_outside_an_outgoing_only_preview() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = AppStore::new(open_database(tmp.path()).unwrap());
        assert!(store.incoming_message_ids("chat").unwrap().is_empty());
        for (id, at, outgoing, system) in [
            ("older", 99, false, false),
            ("boundary-seen", 100, false, false),
            ("boundary-unseen", 100, false, false),
            ("newer", 101, false, false),
            ("newer-notice", 101, false, true),
            ("outgoing", 102, true, false),
        ] {
            let mut message = sample_message(id, at, outgoing);
            if system {
                message.kind = ChatMessageKind::System;
            }
            store
                .upsert_notification_preview_message("chat", 5, at, &message)
                .unwrap();
        }
        let mut partial_thread = ThreadRecord {
            chat_id: "chat".to_string(),
            unread_count: 5,
            updated_at_secs: 102,
            messages: vec![sample_message("outgoing", 102, true)],
            draft: "keep draft".to_string(),
        };
        assert_eq!(
            store
                .save_chat_read_state("chat", &sample_read_state(), Some(&partial_thread))
                .unwrap(),
            Some(4)
        );
        let stored = store.load_thread("chat", 1).unwrap().unwrap();
        assert_eq!(stored.unread_count, 4);
        assert_eq!(stored.messages.len(), 1);
        assert_eq!(stored.messages[0].id, "outgoing");
        assert!(matches!(
            stored.messages[0].delivery,
            PersistedDeliveryState::Sent
        ));
        assert_eq!(stored.draft, "keep draft");

        let ids = store.incoming_message_ids("chat").unwrap();
        assert_eq!(ids.len(), 5);
        let mut full_read = sample_read_state();
        full_read.seen_messages = store
            .load_thread("chat", 100)
            .unwrap()
            .unwrap()
            .messages
            .iter()
            .filter(|message| ids.contains(&message.id))
            .map(|message| {
                (
                    message.id.clone(),
                    MessageReadState {
                        created_at_secs: message.created_at_secs,
                        expires_at_secs: message.expires_at_secs,
                    },
                )
            })
            .collect();
        partial_thread.unread_count = 4;
        assert_eq!(
            store
                .save_chat_read_state("chat", &full_read, Some(&partial_thread))
                .unwrap(),
            Some(0)
        );
        let stored = store.load_thread("chat", 100).unwrap().unwrap();
        assert_eq!(stored.unread_count, 0);
        assert_eq!(stored.messages.len(), 6);
        assert!(stored
            .messages
            .iter()
            .filter(|message| !message.is_outgoing)
            .all(|message| matches!(message.delivery, PersistedDeliveryState::Seen)));
    }

    #[test]
    fn chat_read_state_does_not_downgrade_seen_from_a_stale_loaded_projection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = AppStore::new(open_database(tmp.path()).unwrap());
        let stale_message = sample_message("already-seen-newer", 200, false);
        let mut seen_message = stale_message.clone();
        seen_message.delivery = DeliveryState::Seen;
        store
            .upsert_notification_preview_message("chat", 0, 200, &seen_message)
            .unwrap();
        let stale_thread = ThreadRecord {
            chat_id: "chat".to_string(),
            unread_count: 0,
            updated_at_secs: 200,
            messages: vec![stale_message],
            draft: String::new(),
        };
        assert_eq!(
            store
                .save_chat_read_state("chat", &sample_read_state(), Some(&stale_thread))
                .unwrap(),
            Some(0)
        );
        let stored = store.load_thread("chat", 100).unwrap().unwrap();
        assert!(matches!(
            stored.messages[0].delivery,
            PersistedDeliveryState::Seen
        ));
    }

    #[test]
    fn chat_read_pending_markers_are_deduplicated_consumed_and_pruned() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = AppStore::new(open_database(tmp.path()).unwrap());
        let pending = |store: &AppStore| {
            store
                .conn
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM pending_message_reads", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap()
        };
        let state = sample_read_state();
        for _ in 0..5 {
            store.save_chat_read_state("chat", &state, None).unwrap();
        }
        assert_eq!(pending(&store), 1);
        store
            .upsert_notification_preview_message(
                "chat",
                0,
                100,
                &sample_message("boundary-seen", 100, false),
            )
            .unwrap();
        assert_eq!(
            pending(&store),
            0,
            "the original's Seen flag replaces the pending marker"
        );
        assert!(store
            .message_read_state("chat", "boundary-seen")
            .unwrap()
            .is_some());
        assert!(matches!(
            store.load_thread("chat", 10).unwrap().unwrap().messages[0].delivery,
            PersistedDeliveryState::Seen
        ));
        store
            .delete_message_locally("chat", "boundary-seen", None)
            .unwrap();
        store.save_chat_read_state("chat", &state, None).unwrap();
        assert_eq!(
            pending(&store),
            0,
            "replay must not recreate a locally deleted read marker"
        );
        assert!(store.message_read_page(None).unwrap().is_empty());

        let expires = unix_now().get() + 10;
        let mut expiring = state.clone();
        expiring
            .seen_messages
            .get_mut("boundary-seen")
            .unwrap()
            .expires_at_secs = Some(expires);
        store
            .save_chat_read_state("expiring", &expiring, None)
            .unwrap();
        assert_eq!(pending(&store), 1);
        store.delete_expired_messages(expires + 1).unwrap();
        assert_eq!(pending(&store), 0);
        store
            .save_chat_read_state("deleted-chat", &state, None)
            .unwrap();
        store
            .apply_chat_deletion("deleted-chat", 100, false, false)
            .unwrap();
        assert_eq!(pending(&store), 0);
    }

    #[test]
    fn chat_read_upgrade_preserves_only_explicit_legacy_pending_ids() {
        let tmp = tempfile::TempDir::new().unwrap();
        let shared = open_database(tmp.path()).unwrap();
        let mut conn = shared.lock().unwrap();
        conn.execute("INSERT INTO app_meta(key,value) VALUES ('chat_read_state:chat',?1)",
            [serde_json::json!({"updatedAtMs":200000,"deviceId":"device-a", "seenThroughSecs":100,"seenAtBoundary":["boundary-seen"]}).to_string()]).unwrap();
        conn.execute("DROP TABLE pending_message_reads", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 43).unwrap();
        super::super::super::schema::ensure_schema(&mut conn).unwrap();
        drop(conn);
        let mut store = AppStore::new(shared);
        assert_eq!(store.message_read_page(None).unwrap().len(), 1);
        for (id, at) in [("older-unseen", 99), ("boundary-seen", 100)] {
            store
                .upsert_notification_preview_message("chat", 1, at, &sample_message(id, at, false))
                .unwrap();
        }
        assert!(store
            .message_read_state("chat", "boundary-seen")
            .unwrap()
            .is_some());
        assert!(store
            .message_read_state("chat", "older-unseen")
            .unwrap()
            .is_none());
        let batch = &store.load_chat_read_states().unwrap()["chat"];
        assert_eq!(
            batch.seen_through_secs, 0,
            "never retransmit a legacy cutoff"
        );
        assert!(!batch.covers(99, "older-unseen"));
    }
}
