use super::*;
use nostr::{EventBuilder, Keys, Kind};
use std::sync::{Arc, Mutex};

struct Fixture {
    store: AppStore,
    replay: GroupEnvelopeReplay,
    owner: Keys,
    author: Keys,
}

impl Fixture {
    fn new() -> Self {
        let owner = Keys::generate();
        let author = Keys::generate();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        super::super::schema::ensure_schema(&mut conn).unwrap();
        let mut store = AppStore::new(Arc::new(Mutex::new(conn)));
        store.bind_account(owner.public_key()).unwrap();
        store
            .shared()
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO groups(group_id, group_json) VALUES ('test', '{}');
                 INSERT INTO threads(chat_id) VALUES ('group:test');",
            )
            .unwrap();
        let groups = BTreeMap::from([(
            "group:test".into(),
            BTreeSet::from([author.public_key().to_hex()]),
        )]);
        let replay = GroupEnvelopeReplay::new(store.shared(), owner.public_key().to_hex(), groups);
        Self {
            store,
            replay,
            owner,
            author,
        }
    }

    fn event(&self, body: &str) -> Event {
        EventBuilder::new(Kind::Custom(MESSAGE_EVENT_KIND as u16), body)
            .sign_with_keys(&self.author)
            .unwrap()
    }

    fn add(&self, event: &Event) {
        self.store.shared().lock().unwrap().execute(
            "INSERT OR IGNORE INTO messages
             (chat_id,id,kind,author,author_owner_pubkey_hex,body,is_outgoing,created_at_secs,delivery,source_event_id)
             VALUES ('group:test',?1,'user',?2,?2,'private decrypted text',0,?3,'received',?1)",
            params![event.id.to_hex(),self.author.public_key().to_hex(),unix_now().get() as i64],
        ).unwrap();
        self.store
            .retain_group_envelope(
                &self.owner.public_key().to_hex(),
                "group:test",
                &event.id.to_hex(),
                event,
            )
            .unwrap();
    }

    fn count(&self) -> usize {
        self.store
            .shared()
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM mesh_group_envelopes", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn query(&self, event: &Event) -> QueryReport {
        self.replay
            .try_read(&[Filter::new().id(event.id)], 64)
            .unwrap()
            .unwrap()
    }
}

#[test]
fn group_replay_retains_exact_signed_ciphertext_with_bounded_count_and_bytes() {
    let f = Fixture::new();
    let first = f.event("opaque ciphertext first");
    f.add(&first);
    let report = f.query(&first);
    assert_eq!(report.events.len(), 1);
    assert_eq!(report.events[0].event.as_event(), &first);
    let stored: String = f
        .store
        .shared()
        .lock()
        .unwrap()
        .query_row("SELECT event_json FROM mesh_group_envelopes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!stored.contains("private decrypted text"));
    // Local attachment metadata/cache updates are not message edits and must
    // not destroy the ciphertext needed by another member after restart.
    f.store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET attachments_json='[{\"localCache\":true}]' WHERE id=?1",
            [first.id.to_hex()],
        )
        .unwrap();
    assert_eq!(f.query(&first).events.len(), 1);
    for index in 0..MAX_ENVELOPES {
        f.add(&f.event(&format!("opaque ciphertext {index}")));
    }
    assert_eq!(f.count(), MAX_ENVELOPES);
    assert!(f.query(&first).events.is_empty());
    let large = f.event(&"x".repeat(MAX_EVENT_BYTES));
    f.add(&large);
    assert_eq!(f.count(), MAX_ENVELOPES);
    assert!(f.query(&large).events.is_empty());
    let latest = f.event("last ciphertext");
    f.add(&latest);
    f.add(&latest);
    assert_eq!(f.count(), MAX_ENVELOPES);
    assert_eq!(f.query(&latest).events.len(), 1);
}

#[test]
fn group_replay_rejects_forgery_private_publication_wildcards_and_wrong_account() {
    let f = Fixture::new();
    let event = f.event("opaque ciphertext");
    f.add(&event);
    assert!(f
        .replay
        .try_read(&[Filter::new()], 64)
        .unwrap()
        .unwrap()
        .events
        .is_empty());
    assert!(f
        .replay
        .try_read(&[Filter::new().id(event.id)], 0)
        .unwrap()
        .unwrap()
        .events
        .is_empty());
    let denied = GroupEnvelopeReplay::new(
        f.store.shared(),
        f.owner.public_key().to_hex(),
        BTreeMap::new(),
    );
    assert!(denied
        .try_read(&[Filter::new().id(event.id)], 64)
        .unwrap()
        .unwrap()
        .events
        .is_empty());
    let other = GroupEnvelopeReplay::new(
        f.store.shared(),
        Keys::generate().public_key().to_hex(),
        f.replay.groups.clone(),
    );
    assert!(other.try_read(&[Filter::new().id(event.id)], 64).is_err());
    let private = EventBuilder::new(Kind::Custom(37368), "private account setting")
        .sign_with_keys(&f.owner)
        .unwrap();
    assert!(f
        .store
        .retain_group_envelope(
            &f.owner.public_key().to_hex(),
            "group:test",
            "private",
            &private
        )
        .is_err());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let report = runtime
        .block_on(f.replay.publish(
            VerifiedEvent::try_from(event.clone()).unwrap(),
            EventSource::local_index("untrusted"),
        ))
        .unwrap();
    assert!(!report.accepted);
    let mut forged = event.clone();
    forged.content.push_str("changed");
    assert!(f
        .store
        .retain_group_envelope(
            &f.owner.public_key().to_hex(),
            "group:test",
            "forged",
            &forged
        )
        .is_err());
    f.store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "UPDATE mesh_group_envelopes SET event_json=?1",
            [forged.as_json()],
        )
        .unwrap();
    assert!(f.query(&event).events.is_empty());
}

#[test]
fn group_replay_tracks_expiry_edits_message_chat_deletion_and_account_clear() {
    let mut f = Fixture::new();
    let event = f.event("expires");
    f.add(&event);
    f.store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET expires_at_secs=1 WHERE id=?1",
            [event.id.to_hex()],
        )
        .unwrap();
    assert!(f.query(&event).events.is_empty());
    for (body, mutation) in [
        (
            "edited",
            "UPDATE messages SET body='edited message' WHERE id=?1",
        ),
        ("deleted", "DELETE FROM messages WHERE id=?1"),
    ] {
        let event = f.event(body);
        f.add(&event);
        assert_eq!(f.query(&event).events.len(), 1);
        f.store
            .shared()
            .lock()
            .unwrap()
            .execute(mutation, [event.id.to_hex()])
            .unwrap();
        assert!(f.query(&event).events.is_empty());
        let remains: bool = f
            .store
            .shared()
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mesh_group_envelopes WHERE event_id=?1)",
                [event.id.to_hex()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!remains);
    }
    let event = f.event("chat deletion");
    f.add(&event);
    f.store
        .apply_chat_deletion("group:test", unix_now().get(), false, false)
        .unwrap();
    assert!(f.query(&event).events.is_empty());
    assert_eq!(f.count(), 0);
    f.store.clear().unwrap();
    assert_eq!(f.count(), 0);
}

#[test]
fn group_replay_respects_durable_block_intervals_after_unblock() {
    let f = Fixture::new();
    let event = f.event("received before a later block was synced");
    f.add(&event);
    let intervals = serde_json::json!([{"author": f.author.public_key().to_hex(), "since": 0, "until": unix_now().get()+1}]);
    f.store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO app_meta(key,value) VALUES (?1,?2)",
            params![
                format!(
                    "iris-chat-private-block-intervals-v1:{}",
                    f.owner.public_key().to_hex()
                ),
                intervals.to_string()
            ],
        )
        .unwrap();
    assert!(f.query(&event).events.is_empty());
}

#[test]
fn group_replay_storage_contention_is_cancellable_bounded_and_recovers() {
    let f = Fixture::new();
    let event = f.event("available after foreground persistence");
    f.add(&event);
    let conn = f.store.shared();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    // A real competing writer holds the same connection. The watchdog bounds
    // failure even if a regression blocks this single-thread async executor.
    let holder = std::thread::spawn(move || {
        let _guard = conn.lock().unwrap();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(3));
    });
    ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let query = || {
            f.replay.query(
                vec![Filter::new().id(event.id)],
                QueryOptions { limit: Some(64) },
            )
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(20), query())
                .await
                .is_err(),
            "transport cancellation must interrupt the storage wait"
        );
        let error = tokio::time::timeout(Duration::from_millis(500), query())
            .await
            .expect("the replay source must bound its own storage wait")
            .expect_err("a held connection must report contention");
        assert!(error.to_string().contains("group replay storage busy"));
        release_tx.send(()).unwrap();
        // Retry can recover while the holder is dropping its guard; there is
        // no detached query or blocking worker left behind by cancellation.
        let report = tokio::time::timeout(Duration::from_millis(500), query())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.events[0].event.as_event(), &event);
    });
    holder.join().unwrap();
}
