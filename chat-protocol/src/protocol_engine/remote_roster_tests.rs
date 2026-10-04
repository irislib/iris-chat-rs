mod remote_roster_tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct CountedAllocator;

    thread_local! {
        // Count only the measured synchronous operation, never other test threads.
        static ALLOCATED_BYTES: Cell<Option<usize>> = const { Cell::new(None) };
    }

    fn record_allocation(size: usize) {
        let _ = ALLOCATED_BYTES.try_with(|bytes| {
            if let Some(current) = bytes.get() {
                bytes.set(Some(current.saturating_add(size)));
            }
        });
    }

    // SAFETY: Every allocation and deallocation is forwarded unchanged to System.
    unsafe impl GlobalAlloc for CountedAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let result = unsafe { System.alloc(layout) };
            if !result.is_null() {
                record_allocation(layout.size());
            }
            result
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let result = unsafe { System.alloc_zeroed(layout) };
            if !result.is_null() {
                record_allocation(layout.size());
            }
            result
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let result = unsafe { System.realloc(ptr, layout, size) };
            if !result.is_null() {
                record_allocation(size);
            }
            result
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) };
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountedAllocator = CountedAllocator;

    fn allocated_bytes<T>(operation: impl FnOnce() -> T) -> (T, usize) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                ALLOCATED_BYTES.with(|bytes| bytes.set(None));
            }
        }
        ALLOCATED_BYTES.with(|bytes| assert!(bytes.replace(Some(0)).is_none()));
        let reset = Reset;
        let result = operation();
        let bytes = ALLOCATED_BYTES.with(|bytes| bytes.get().unwrap());
        drop(reset);
        (result, bytes)
    }

    #[test]
    fn large_checkpoint_comparison_does_not_allocate_previous_value() {
        let dir = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("state.sqlite3")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; CREATE TABLE ndr_kv (
            owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL,
            key TEXT NOT NULL, value TEXT NOT NULL,
            PRIMARY KEY(owner_pubkey_hex, device_pubkey_hex, key));",
        )
        .unwrap();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(conn));
        let adapter = crate::SqliteStorageAdapter::new(shared, "owner".into(), "device".into());
        let original = "a".repeat(8 * 1024 * 1024 + 31);
        adapter.put("checkpoint", original.clone()).unwrap();
        let mut changed = original.clone();
        changed.replace_range(0..1, "b");
        let last = changed.len() - 1;
        changed.replace_range(last..last + 1, "c");

        for (case, value) in [("equal", original), ("changed", changed)] {
            // Input construction and the verification read are outside the
            // measured production put. SQLite's C allocations are not counted.
            let expected = value.clone();
            let (result, bytes) = allocated_bytes(|| adapter.put("checkpoint", value));
            result.unwrap();
            eprintln!("{case} checkpoint comparison allocated {bytes} Rust bytes");
            assert!(
                bytes < 64 * 1024,
                "{case} comparison allocated {bytes} Rust bytes for an {}-byte checkpoint",
                expected.len()
            );
            assert_eq!(adapter.get("checkpoint").unwrap(), Some(expected));
        }
    }

    fn pending_for(owner: PublicKey, device: PublicKey) -> ProtocolPendingRemoteSend {
        ProtocolPendingRemoteSend {
            recipient_owner: ndr_owner(owner),
            eligible_devices: BTreeSet::from([ndr_device(device)]),
            completed_devices: BTreeSet::new(),
            chat_id: owner.to_hex(),
            payload: b"pending message".to_vec(),
            inner_event_id: None,
            message_id: "pending".into(),
            expires_at_secs: None,
            created_at_secs: 10,
            next_retry_at_secs: 10,
        }
    }

    fn queue_error(engine: &mut ProtocolEngine, peer: PublicKey) -> String {
        engine
            .queue_remote_payload(
                peer,
                "chat",
                b"message".to_vec(),
                None,
                "message",
                UnixSeconds(10),
            )
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn remote_roster_missing_waits_but_empty_revokes_pending_targets() {
        let mut f = remote_send_fixture();
        let peer = Keys::generate().public_key();
        let mut pending = pending_for(peer, f.late_device.public_key());
        let original_targets = pending.eligible_devices.clone();
        assert_eq!(
            queue_error(&mut f.sender, peer),
            "missing recipient device list"
        );
        let (effects, complete) = f
            .sender
            .prepare_pending_remote_send(&mut pending, NdrUnixSeconds(20))
            .unwrap();
        assert!(effects.is_empty());
        assert!(!complete, "a missing roster must leave the message pending");
        assert_eq!(pending.eligible_devices, original_targets);

        f.sender.session_manager.observe_peer_roster(
            ndr_owner(peer),
            DeviceRoster::new(NdrUnixSeconds(1), Vec::new()),
        );
        assert_eq!(
            queue_error(&mut f.sender, peer),
            "no remote target prepared"
        );
        let (effects, complete) = f
            .sender
            .prepare_pending_remote_send(&mut pending, NdrUnixSeconds(30))
            .unwrap();
        assert!(effects.is_empty());
        assert!(complete, "an explicitly empty roster revokes every target");
        assert!(pending.eligible_devices.is_empty());
    }

    #[test]
    fn remote_roster_lookup_allocations_do_not_scale_with_unrelated_sessions() {
        let mut f = remote_send_fixture();
        let peer = f.peer_owner.public_key();
        f.sender
            .send_direct_text(peer, &peer.to_hex(), "seed", None, UnixSeconds(10))
            .unwrap();
        let missing_peer = Keys::generate().public_key();
        let mut pending = pending_for(missing_peer, f.late_device.public_key());
        let (_, initial_queue_bytes) = allocated_bytes(|| queue_error(&mut f.sender, missing_peer));
        let ((effects, complete), initial_retry_bytes) = allocated_bytes(|| {
            f.sender
                .prepare_pending_remote_send(&mut pending, NdrUnixSeconds(20))
                .unwrap()
        });
        assert!(effects.is_empty() && !complete);

        let mut state = f
            .sender
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .flat_map(|user| user.devices)
            .find_map(|device| device.active_session)
            .expect("the initial delivery established a synthetic session");
        state.skipped_keys.insert(
            ndr_device(Keys::generate().public_key()),
            nostr_double_ratchet::SkippedKeysEntry {
                message_keys: (0..256).map(|number| (number, [7; 32])).collect(),
            },
        );
        for _ in 0..16 {
            let unrelated = Keys::generate().public_key();
            f.sender.session_manager.import_session_state(
                ndr_owner(unrelated),
                ndr_device(unrelated),
                state.clone(),
                NdrUnixSeconds(10),
            );
        }
        let snapshot = f.sender.session_manager.snapshot();
        let skipped_keys: usize = snapshot
            .users
            .iter()
            .flat_map(|user| &user.devices)
            .filter_map(|device| device.active_session.as_ref())
            .flat_map(|state| state.skipped_keys.values())
            .map(|entry| entry.message_keys.len())
            .sum();
        assert!(
            skipped_keys >= 4096,
            "the synthetic workload must be retained"
        );

        let (error, loaded_queue_bytes) =
            allocated_bytes(|| queue_error(&mut f.sender, missing_peer));
        let ((effects, complete), loaded_retry_bytes) = allocated_bytes(|| {
            f.sender
                .prepare_pending_remote_send(&mut pending, NdrUnixSeconds(30))
                .unwrap()
        });
        assert_eq!(error, "missing recipient device list");
        assert!(effects.is_empty() && !complete);
        eprintln!(
            "recipient lookup allocated bytes: queue {initial_queue_bytes}->{loaded_queue_bytes}, retry {initial_retry_bytes}->{loaded_retry_bytes}"
        );
        // Allow small unrelated bookkeeping changes, but no copying of the
        // thousands of skipped keys belonging to other conversations.
        assert!(
            loaded_queue_bytes <= initial_queue_bytes + 4096,
            "new-send lookup copied unrelated session state"
        );
        assert!(
            loaded_retry_bytes <= initial_retry_bytes + 4096,
            "retry lookup copied unrelated session state"
        );
    }
}
