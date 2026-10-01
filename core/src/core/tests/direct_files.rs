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
    let mut message = core.threads[chat]
        .messages
        .iter()
        .find(|m| m.body.starts_with("iris-direct-file-v1:") && m.body.contains(id))
        .expect("signed offer in message history")
        .clone();
    super::direct_files::decorate(
        &mut message,
        core.state.account.as_ref(),
        &core.app_store.shared(),
    );
    message
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

fn exercise_direct_files_actions(same_owner: bool) {
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
    let a_chat = bo.public_key().to_hex();
    let b_chat = ao.public_key().to_hex();
    if same_owner {
        configure_test_device_sync_profile(&mut a, &ao, &ad, &bd, None);
        configure_test_device_sync_profile(&mut b, &ao, &bd, &ad, None);
        install_two_way_local_sibling_state_for_test(&mut a, &mut b, &ao, &ad, &bd);
        let roster_at = unix_now().get();
        let roster = AppKeys::new(vec![
            DeviceEntry::new(ad.public_key(), roster_at),
            DeviceEntry::new(bd.public_key(), roster_at),
        ]);
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
    let addr = || {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    };
    let aa = addr();
    let ba = addr();
    a.reconcile_calls_udp_for_test(aa, ba, &test_fips_peer(&bd).npub());
    b.reconcile_calls_udp_for_test(ba, aa, &test_fips_peer(&ad).npub());
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
    b.handle_action(AppAction::AcceptDirectFiles {
        chat_id: b_chat.clone(),
        transfer_id: id.clone(),
    });
    direct_files_wait(&mut a, &ar, &mut b, &br, |a, b| {
        [a, b].iter().all(|core| {
            direct_files_test_records(core)
                .iter()
                .any(|r| r.offer.id == id && r.status == crate::DirectFileTransferStatus::Completed)
        })
    });
    let result = direct_files_test_message(&b, &b_chat, &id)
        .direct_transfer
        .unwrap();
    assert!(!result.is_sender);
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
    assert_eq!(
        std::fs::read(input_dir.join(names[0])).unwrap(),
        b"changed original"
    );
    assert!(a.logged_in.as_ref().unwrap().relay_urls.is_empty());
    assert!(b.logged_in.as_ref().unwrap().relay_urls.is_empty());
    a.stop_device_sync_now();
    b.stop_device_sync_now();
}

#[test]
fn direct_files_actions_copy_sign_offer_and_receive_multiple_files() {
    exercise_direct_files_actions(false);
}

#[test]
fn direct_files_self_chat_outgoing_offer_is_received_on_another_device() {
    exercise_direct_files_actions(true);
}
