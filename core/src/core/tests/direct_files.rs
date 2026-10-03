fn direct_files_test_records(core: &AppCore) -> Vec<super::direct_files::Record> {
    let db = core.app_store.shared();
    let conn = db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT record_json FROM direct_file_transfers ORDER BY id")
        .unwrap();
    let records = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|json| serde_json::from_str(&json.unwrap()).unwrap())
        .collect();
    records
}

fn direct_files_test_message(core: &AppCore, chat: &str, id: &str) -> ChatMessageSnapshot {
    chat_snapshot_from_state_and_db(&core.state, Some(&core.app_store.shared()), chat, 100)
        .expect("chat snapshot")
        .messages
        .into_iter()
        .find(|m| {
            m.direct_transfer
                .as_ref()
                .is_some_and(|transfer| transfer.id == id)
        })
        .expect("direct file card in actual app history snapshot")
}

fn direct_files_wait(
    a: &mut AppCore,
    ar: &flume::Receiver<CoreMsg>,
    b: &mut AppCore,
    br: &flume::Receiver<CoreMsg>,
    predicate: impl Fn(&AppCore, &AppCore) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        pump_call_pair(a, ar, b, br);
        if predicate(a, b) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "file actions did not converge: {:?} / {:?}; errors={:?}/{:?}",
            direct_files_test_records(a)
                .iter()
                .map(|r| &r.status)
                .collect::<Vec<_>>(),
            direct_files_test_records(b)
                .iter()
                .map(|r| &r.status)
                .collect::<Vec<_>>(),
            a.state.toast,
            b.state.toast
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn exercise_direct_files_actions(same_owner: bool, outcome: crate::DirectFileTransferStatus) {
    use sha2::{Digest, Sha256};
    let ao = Keys::generate();
    let bo = if same_owner {
        ao.clone()
    } else {
        Keys::generate()
    };
    let ad = Keys::generate();
    let bd = Keys::generate();
    let (mut a, _au, adir) = logged_in_test_core_with_updates("direct-files-a", &ao, &ad);
    let (mut b, _bu, bdir) = logged_in_test_core_with_updates("direct-files-b", &bo, &bd);
    for core in [&mut a, &mut b] {
        core.preferences.nostr_relay_urls.clear();
        core.preferences.nearby_enabled = false;
    }
    let a_chat = bo.public_key().to_hex();
    let b_chat = ao.public_key().to_hex();
    let mut signed_self_roster = None;
    if same_owner {
        configure_test_device_sync_profile(&mut a, &ao, &ad, &bd, None);
        configure_test_device_sync_profile(&mut b, &ao, &bd, &ad, None);
        install_two_way_local_sibling_state_for_test(&mut a, &mut b, &ao, &ad, &bd);
        let roster_at = unix_now().get();
        let roster = AppKeys::new(vec![
            DeviceEntry::new(ad.public_key(), roster_at),
            DeviceEntry::new(bd.public_key(), roster_at),
        ]);
        signed_self_roster = Some(
            roster
                .get_event_at(ao.public_key(), roster_at)
                .sign_with_keys(&ao)
                .expect("signed self-device roster"),
        );
        for core in [&mut a, &mut b] {
            observe_peer_appkeys_for_test(
                core.protocol_engine.as_mut().unwrap(),
                &ao,
                &[ad.public_key(), bd.public_key()],
                roster_at,
            );
            core.app_keys.insert(
                b_chat.clone(),
                known_app_keys_from_ndr(ao.public_key(), &roster, roster_at),
            );
            assert!(
                core.protocol_engine
                    .as_ref()
                    .unwrap()
                    .direct_send_readiness(ao.public_key())
                    .is_ready(),
                "self-chat fixture needs signed local device authorization"
            );
            core.ensure_thread_record(&b_chat, unix_now().get());
        }
    } else {
        for core in [&mut a, &mut b] {
            call_test_peer(core, &ao, &ad);
            call_test_peer(core, &bo, &bd);
        }
    }
    let (at, ar) = flume::unbounded();
    a.core_sender = at.clone();
    a.priority_sender = at;
    let (bt, br) = flume::unbounded();
    b.core_sender = bt.clone();
    b.priority_sender = bt;
    // Signed roster events run normal reconciliation while an offer is active.
    // Use production discovery on both peers: a test-only UDP configuration has
    // a different runtime key and would be replaced by that reconciliation.
    a.reconcile_device_sync();
    b.reconcile_device_sync();
    let ae = a.device_sync_endpoint_for_test().unwrap();
    let be = b.device_sync_endpoint_for_test().unwrap();
    direct_files_wait(&mut a, &ar, &mut b, &br, |a, _| {
        a.runtime.block_on(device_sync_pair_is_connected([
            (&ae, &test_fips_peer(&bd)),
            (&be, &test_fips_peer(&ad)),
        ]))
    });
    let contents = [
        b"private copied payload".to_vec(),
        vec![0x8f; 130_123],
        Vec::new(),
    ];
    let names = ["notes.txt", "photo.bin", "empty.txt"];
    let input_dir = adir.path().join("selected");
    std::fs::create_dir(&input_dir).unwrap();
    let attachments = names
        .iter()
        .zip(&contents)
        .map(|(name, bytes)| {
            let path = input_dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            OutgoingAttachment {
                filename: (*name).into(),
                file_path: path.to_string_lossy().into_owned(),
            }
        })
        .collect();
    a.handle_action(AppAction::SendDirectFiles {
        chat_id: a_chat.clone(),
        attachments,
        caption: "Files for you".into(),
    });
    direct_files_wait(&mut a, &ar, &mut b, &br, |a, _| {
        !direct_files_test_records(a).is_empty() && !a.state.busy.sending_message
    });
    let record = direct_files_test_records(&a).remove(0);
    assert!(record.is_sender);
    assert_eq!(record.status, crate::DirectFileTransferStatus::Offered);
    assert_eq!(record.offer.files.len(), 3);
    assert_eq!(record.offer.device, ad.public_key().to_hex());
    if let Some(roster) = &signed_self_roster {
        // Re-deliver the unchanged signed roster after registration, rather than
        // depending on when the live sibling transport happens to deliver it.
        let generation = a.fips_connection_generation;
        let endpoint = a.device_sync_endpoint_for_test().unwrap();
        assert!(a.apply_app_keys_event(roster).expect("same signed roster"));
        assert_eq!(a.fips_connection_generation, generation);
        assert!(Arc::ptr_eq(
            &endpoint,
            &a.device_sync_endpoint_for_test().unwrap()
        ));
        assert_eq!(
            direct_files_test_records(&a)[0].status,
            crate::DirectFileTransferStatus::Offered,
            "an unchanged roster must retain the registered file capability"
        );
    }
    let signed: Event =
        serde_json::from_str(record.wire.strip_prefix("iris-direct-file-v1:").unwrap()).unwrap();
    signed.verify().expect("offer is signed");
    assert_eq!(signed.pubkey, ad.public_key());
    assert!(!record.wire.contains("private copied payload"));
    assert!(!record
        .wire
        .contains(&input_dir.to_string_lossy().to_string()));
    for ((file, path), expected) in record.offer.files.iter().zip(&record.paths).zip(&contents) {
        assert_eq!(std::fs::read(path).unwrap(), *expected);
        assert_eq!(file.sha256, format!("{:x}", Sha256::digest(expected)));
    }
    std::fs::write(input_dir.join(names[0]), b"changed original").unwrap();
    std::fs::remove_file(input_dir.join(names[1])).unwrap();
    let id = record.offer.id.clone();
    let original = a.threads[&a_chat]
        .messages
        .iter()
        .find(|m| m.body == record.wire)
        .unwrap()
        .clone();
    if same_owner {
        // The live sibling connection must announce the new offer without a
        // message server or a reconnect/history request.
        direct_files_wait(&mut a, &ar, &mut b, &br, |_, b| {
            b.threads.get(&b_chat).is_some_and(|thread| {
                thread
                    .messages
                    .iter()
                    .any(|message| message.body == record.wire)
            })
        });
        // Cross the real sibling-sync encoder and authenticated packet ingestion.
        // The message is outgoing on both devices; only its source owns the offer.
        for packet in a.build_device_sync_packets_for_test(100, true) {
            b.handle_device_sync_packet(&ad.public_key().to_hex(), DEVICE_SYNC_PORT, &packet);
        }
    } else {
        // Production decrypted-message ingestion receives the exact signed offer
        // that SendDirectFiles placed in encrypted chat.
        b.apply_runtime_text_message(
            ao.public_key(),
            Some(b_chat.clone()),
            record.wire.clone(),
            original.created_at_secs,
            None,
            Some(original.id),
            None,
        );
    }
    b.rebuild_persist_and_emit_state();
    b.handle_action(AppAction::OpenChat {
        chat_id: b_chat.clone(),
    });
    let message = direct_files_test_message(&b, &b_chat, &id);
    assert_eq!(message.is_outgoing, same_owner);
    assert!(
        message.attachments.is_empty(),
        "direct files never enter blob attachments"
    );
    let offer = message.direct_transfer.unwrap();
    assert!(
        !offer.is_sender,
        "outgoing self-chat offer must be receivable by another device"
    );
    assert_eq!(offer.status, crate::DirectFileTransferStatus::Offered);
    assert!(offer.files.iter().all(|f| f.local_path.is_none()));
    let receiving = bdir.path().join("direct-files").join(&id);
    let waiting = std::time::Instant::now() + Duration::from_millis(150);
    while std::time::Instant::now() < waiting {
        pump_call_pair(&mut a, &ar, &mut b, &br);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!receiving.exists(), "no received files before acceptance");
    assert_eq!(
        direct_files_test_records(&a)[0].status,
        crate::DirectFileTransferStatus::Offered
    );
    match outcome {
        crate::DirectFileTransferStatus::Failed => {
            // The offer is already in both histories. Losing a staged source
            // produces a real mid-batch transport failure after the first file.
            std::fs::remove_file(&record.paths[1]).unwrap();
            b.handle_action(AppAction::AcceptDirectFiles {
                chat_id: b_chat.clone(),
                transfer_id: id.clone(),
            });
        }
        crate::DirectFileTransferStatus::Cancelled => {
            for (core, chat) in [(&mut a, &a_chat), (&mut b, &b_chat)] {
                core.handle_action(AppAction::CancelDirectFiles {
                    chat_id: chat.clone(),
                    transfer_id: id.clone(),
                });
            }
        }
        crate::DirectFileTransferStatus::Completed => {
            b.handle_action(AppAction::AcceptDirectFiles {
                chat_id: b_chat.clone(),
                transfer_id: id.clone(),
            });
        }
        _ => unreachable!("unsupported test outcome"),
    }
    direct_files_wait(&mut a, &ar, &mut b, &br, |a, b| {
        [a, b].iter().all(|core| {
            direct_files_test_records(core)
                .iter()
                .any(|r| r.offer.id == id && r.status == outcome)
        })
    });
    let result = direct_files_test_message(&b, &b_chat, &id)
        .direct_transfer
        .unwrap();
    assert!(!result.is_sender);
    if outcome == crate::DirectFileTransferStatus::Completed {
        assert_eq!(
            result.transferred_bytes,
            contents.iter().map(|v| v.len() as u64).sum::<u64>()
        );
        for (file, expected) in result.files.iter().zip(&contents) {
            assert_eq!(
                std::fs::read(file.local_path.as_ref().unwrap()).unwrap(),
                *expected
            );
        }
    }
    assert_eq!(
        std::fs::read(input_dir.join(names[0])).unwrap(),
        b"changed original"
    );
    assert!(a.logged_in.as_ref().unwrap().relay_urls.is_empty());
    assert!(b.logged_in.as_ref().unwrap().relay_urls.is_empty());
    a.stop_device_sync_now();
    b.stop_device_sync_now();
    let a_error = direct_files_test_message(&a, &a_chat, &id)
        .direct_transfer
        .unwrap()
        .error;
    let b_error = direct_files_test_message(&b, &b_chat, &id)
        .direct_transfer
        .unwrap()
        .error;
    // Removing local copies must not remove the message or change its outcome.
    for dir in [&adir, &bdir] {
        let files = dir.path().join("direct-files");
        if files.exists() {
            std::fs::remove_dir_all(files).unwrap();
        }
    }
    for (core, chat) in [(&mut a, &a_chat), (&mut b, &b_chat)] {
        core.rebuild_state();
        let transfer = direct_files_test_message(core, chat, &id)
            .direct_transfer
            .unwrap();
        assert_eq!(transfer.status, outcome);
        assert!(transfer.files.iter().all(|file| file.local_path.is_none()));
    }
    a = reopen_direct_file_history(a, &ao, &ad);
    b = reopen_direct_file_history(b, &bo, &bd);
    for (core, chat, error, is_sender) in
        [(&a, &a_chat, a_error, true), (&b, &b_chat, b_error, false)]
    {
        let message = direct_files_test_message(core, chat, &id);
        assert_eq!(message.body, "Files for you");
        let transfer = message.direct_transfer.unwrap();
        assert_eq!(transfer.status, outcome);
        assert_eq!(transfer.is_sender, is_sender);
        assert_eq!(transfer.error, error);
        assert_eq!(
            transfer
                .files
                .iter()
                .map(|file| file.filename.as_str())
                .collect::<Vec<_>>(),
            names
        );
        assert!(transfer.files.iter().all(|file| file.local_path.is_none()));
        // Paginated/inactive history and the active chat must project the same
        // durable outcome.
        let mut state = core.state.clone();
        state.current_chat = None;
        let page =
            chat_snapshot_from_state_and_db(&state, Some(&core.app_store.shared()), chat, 100)
                .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find_map(|message| message.direct_transfer.as_ref())
                .unwrap()
                .status,
            outcome
        );
        assert!(core.threads[chat]
            .messages
            .iter()
            .any(|message| message.body == record.wire));
    }
    // Durable outcomes never resurrect explicitly deleted or expired messages.
    a.handle_action(AppAction::DeleteChat {
        chat_id: a_chat.clone(),
    });
    a = reopen_direct_file_history(a, &ao, &ad);
    assert!(!a.threads.contains_key(&a_chat));
    let message = b
        .threads
        .get_mut(&b_chat)
        .unwrap()
        .messages
        .iter_mut()
        .find(|message| message.body == record.wire)
        .unwrap();
    message.expires_at_secs = Some(unix_now().get().saturating_sub(1));
    b.persist_best_effort();
    b = reopen_direct_file_history(b, &bo, &bd);
    assert!(!b.threads.get(&b_chat).is_some_and(|thread| thread
        .messages
        .iter()
        .any(|message| message.body == record.wire)));
}

fn reopen_direct_file_history(mut core: AppCore, owner: &Keys, device: &Keys) -> AppCore {
    core.stop_device_sync_now();
    core.persist_best_effort();
    let directory = core.data_dir.to_string_lossy().into_owned();
    drop(core);
    // Keep startup work queued: this regression exercises actual account/SQLite
    // restoration without contacting message servers or mutating restored state.
    let (sender, _pending) = flume::unbounded();
    let mut restored = AppCore::new(
        flume::unbounded().0,
        sender,
        directory,
        Arc::new(RwLock::new(AppState::empty())),
    );
    restored
        .start_session_inner(
            owner.public_key(),
            Some(owner.clone()),
            device.clone(),
            true,
            true,
            false,
        )
        .unwrap();
    restored.stop_device_sync_now();
    restored
}

#[test]
fn direct_files_actions_copy_sign_offer_and_receive_multiple_files() {
    exercise_direct_files_actions(false, crate::DirectFileTransferStatus::Completed);
}

#[test]
fn direct_files_self_chat_outgoing_offer_is_received_on_another_device() {
    exercise_direct_files_actions(true, crate::DirectFileTransferStatus::Completed);
}

#[test]
fn direct_files_failed_batch_remains_in_history_after_restart() {
    exercise_direct_files_actions(false, crate::DirectFileTransferStatus::Failed);
}

#[test]
fn direct_files_cancelled_batch_remains_in_history_after_restart() {
    exercise_direct_files_actions(false, crate::DirectFileTransferStatus::Cancelled);
}

#[test]
fn direct_files_failed_offer_send_keeps_failed_history_after_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let chat = peer_owner.public_key().to_hex();
    let (mut core, _updates, directory) =
        logged_in_test_core_with_updates("file-send-failed", &owner, &device);
    core.preferences.nostr_relay_urls.clear();
    core.preferences.nearby_enabled = false;
    call_test_peer(&mut core, &owner, &device);
    call_test_peer(&mut core, &peer_owner, &peer_device);
    let (sender, pending) = flume::unbounded();
    core.core_sender = sender.clone();
    core.priority_sender = sender;
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    core.reconcile_calls_udp_for_test(
        "127.0.0.1:0".parse().unwrap(),
        silent.local_addr().unwrap(),
        &test_fips_peer(&peer_device).npub(),
    );
    core.app_store.shared().lock().unwrap().execute_batch(
        "CREATE TRIGGER fail_direct_offer_event BEFORE UPDATE OF outgoing_event_json ON messages
         WHEN NEW.body LIKE 'iris-direct-file-v1:%'
         BEGIN SELECT RAISE(FAIL, 'simulated outgoing event storage failure'); END;"
    ).unwrap();
    let path = directory.path().join("selected.txt");
    std::fs::write(&path, b"private file").unwrap();
    core.handle_action(AppAction::SendDirectFiles {
        chat_id: chat.clone(),
        attachments: vec![OutgoingAttachment {
            filename: "selected.txt".into(),
            file_path: path.to_string_lossy().into_owned(),
        }],
        caption: "Keep this failed send".into(),
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while core.state.busy.sending_message || direct_files_test_records(&core).is_empty() {
        for message in pending.try_iter().take(128) {
            core.handle_message(message);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "file preparation should finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let record = direct_files_test_records(&core).remove(0);
    assert_eq!(record.status, crate::DirectFileTransferStatus::Failed);
    assert!(!directory
        .path()
        .join("direct-files")
        .join(&record.offer.id)
        .join("send")
        .exists());
    let message = direct_files_test_message(&core, &chat, &record.offer.id);
    assert_eq!(message.delivery, DeliveryState::Failed);
    assert_eq!(
        message.direct_transfer.unwrap().status,
        crate::DirectFileTransferStatus::Failed
    );
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_direct_offer_event")
        .unwrap();
    core = reopen_direct_file_history(core, &owner, &device);
    let message = direct_files_test_message(&core, &chat, &record.offer.id);
    assert_eq!(message.body, "Keep this failed send");
    assert_eq!(message.delivery, DeliveryState::Failed);
    let transfer = message.direct_transfer.unwrap();
    assert_eq!(transfer.status, crate::DirectFileTransferStatus::Failed);
    assert_eq!(transfer.error, record.error);
    assert!(transfer.files.iter().all(|file| file.local_path.is_none()));
}
