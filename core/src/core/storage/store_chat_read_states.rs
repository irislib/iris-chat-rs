use super::*;
use std::collections::BTreeSet;

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
                states.insert(chat_id.to_string(), serde_json::from_str(&value)?);
            }
        }
        Ok(states)
    }

    pub(crate) fn save_chat_read_state(
        &mut self,
        chat_id: &str,
        state: &ChatReadState,
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
                let already_seen = !message.is_outgoing
                    && !matches!(message.delivery, DeliveryState::Seen)
                    && tx
                        .query_row(
                            "SELECT delivery = 'seen' FROM messages WHERE chat_id = ?1 AND id = ?2",
                            params![chat_id, message.id],
                            |row| row.get::<_, bool>(0),
                        )
                        .optional()?
                        .unwrap_or(false);
                if already_seen {
                    let mut seen = message.clone();
                    seen.delivery = DeliveryState::Seen;
                    upsert_message_row(&tx, chat_id, &seen)?;
                } else {
                    upsert_message_row(&tx, chat_id, message)?;
                }
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
                let mut remaining = 0;
                for row in rows {
                    let (created_at, id) = row?;
                    if !state.covers(created_at.max(0) as u64, &id) {
                        remaining += 1;
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
        if state.seen_through_secs != 0 {
            tx.execute(
                "UPDATE messages SET delivery = 'seen'
                 WHERE chat_id = ?1 AND is_outgoing = 0
                   AND delivery != 'seen' AND created_at_secs < ?2",
                params![chat_id, state.seen_through_secs as i64],
            )?;
            let mut statement = tx.prepare(
                "UPDATE messages SET delivery = 'seen'
                 WHERE chat_id = ?1 AND is_outgoing = 0
                   AND delivery != 'seen' AND created_at_secs = ?2 AND id = ?3",
            )?;
            for message_id in &state.seen_at_boundary {
                statement.execute(params![chat_id, state.seen_through_secs as i64, message_id])?;
            }
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

    pub(crate) fn latest_incoming_read_boundary(
        &self,
        chat_id: &str,
    ) -> anyhow::Result<Option<(u64, BTreeSet<String>)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut statement = conn.prepare(
            "SELECT created_at_secs, id FROM messages
             WHERE chat_id = ?1 AND is_outgoing = 0 AND created_at_secs = (
                 SELECT MAX(created_at_secs) FROM messages WHERE chat_id = ?1 AND is_outgoing = 0
             )",
        )?;
        let rows = statement.query_map([chat_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result: Option<(u64, BTreeSet<String>)> = None;
        for row in rows {
            let (created_at, id) = row?;
            result
                .get_or_insert_with(|| (created_at.max(0) as u64, BTreeSet::new()))
                .1
                .insert(id);
        }
        Ok(result)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::open_database;

    fn sample_read_state() -> ChatReadState {
        ChatReadState {
            updated_at_ms: 12_345,
            device_id: "device-a".to_string(),
            seen_through_secs: 100,
            seen_at_boundary: ["boundary-seen".to_string()].into_iter().collect(),
        }
    }

    fn sample_message(id: &str, created_at_secs: u64, is_outgoing: bool) -> ChatMessageSnapshot {
        ChatMessageSnapshot {
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
                    "INSERT INTO threads(chat_id, unread_count) VALUES (?1, 7)",
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
        assert_eq!(thread.unread_count, 2);
        for message in thread.messages {
            let expected = match message.id.as_str() {
                "older" | "boundary-seen" | "already-seen" | "system" => "seen",
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
        unread.seen_through_secs = 0;
        unread.seen_at_boundary.insert("newer".to_string());
        store.save_chat_read_state("chat", &unread, None).unwrap();
        let thread = store.load_thread("chat", 100).unwrap().unwrap();
        assert!(thread
            .messages
            .iter()
            .filter(|message| matches!(
                message.id.as_str(),
                "older" | "boundary-seen" | "already-seen"
            ))
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
        assert!(store
            .latest_incoming_read_boundary("chat")
            .unwrap()
            .is_none());
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
            Some(3)
        );
        let stored = store.load_thread("chat", 1).unwrap().unwrap();
        assert_eq!(stored.unread_count, 3);
        assert_eq!(stored.messages.len(), 1);
        assert_eq!(stored.messages[0].id, "outgoing");
        assert!(matches!(
            stored.messages[0].delivery,
            PersistedDeliveryState::Sent
        ));
        assert_eq!(stored.draft, "keep draft");

        let (at, boundary) = store
            .latest_incoming_read_boundary("chat")
            .unwrap()
            .unwrap();
        assert_eq!(at, 101);
        assert_eq!(
            boundary,
            ["newer".to_string(), "newer-notice".to_string()]
                .into_iter()
                .collect()
        );
        let mut full_read = sample_read_state();
        full_read.seen_through_secs = at;
        full_read.seen_at_boundary = boundary;
        partial_thread.unread_count = 3;
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
}
