#[test]
fn private_block_event_is_durable_ordered_and_deletes_direct_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let (mut core, _, dir) = logged_in_test_core_with_updates("block-persist", &owner, &device);
    core.push_incoming_message_from(&target, Some("old".into()), "hello".into(), 20, None, None, Some(target.clone()), None);
    core.set_user_blocked(&target, true);
    let blocked = core.private_block_event(&target).unwrap();
    assert!(blocked.verify().is_ok());
    assert!(!core.threads.contains_key(&target));
    assert!(core.is_owner_blocked(&target));
    core.set_user_blocked(&target, false);
    let unblocked = core.private_block_event(&target).unwrap();
    assert!(!core.is_owner_blocked(&target));
    assert!(core.apply_private_block_event(blocked));
    assert_eq!(core.private_block_event(&target).unwrap(), unblocked);
    assert!(!core.is_owner_blocked(&target));
    core.persist_best_effort_inner();
    drop(core);
    let mut restored = logged_in_test_core_at_data_dir(&owner, &device, dir.path().to_string_lossy().into_owned());
    restored.restore_device_sync_record_projection();
    assert!(!restored.is_owner_blocked(&target));
    assert!(!restored.threads.contains_key(&target));
    assert!(restored.chat_activity_is_deleted(&target, 20));
}

#[test]
fn private_block_control_requires_own_authorized_device_and_valid_signature() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let stranger = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut left = logged_in_test_core("block-left", &owner, &a);
    let mut right = logged_in_test_core("block-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    right.app_keys = left.app_keys.clone();
    left.set_user_blocked(&target, true);
    let event = left.private_block_event(&target).unwrap();
    let body = serde_json::json!({"type":"private-block-state","v":1,"event":event}).to_string();
    assert!(right.receive_block_control(stranger.public_key(), Some(a.public_key()), &body));
    assert!(right.receive_block_control(owner.public_key(), Some(stranger.public_key()), &body));
    assert!(right.receive_block_control(owner.public_key(), Some(b.public_key()), &body));
    assert!(!right.is_owner_blocked(&target));
    let mut corrupt = event.clone();
    corrupt.content.push(' ');
    let corrupt = serde_json::json!({"type":"private-block-state","v":1,"event":corrupt}).to_string();
    assert!(right.receive_block_control(owner.public_key(), Some(a.public_key()), &corrupt));
    assert!(!right.is_owner_blocked(&target));
    assert!(right.receive_block_control(owner.public_key(), Some(a.public_key()), &body));
    assert!(right.is_owner_blocked(&target));
    let foreign = Keys::generate();
    let mut foreign_core = logged_in_test_core("block-foreign", &foreign, &Keys::generate());
    assert!(!foreign_core.apply_private_block_event(event));
}

#[test]
fn private_block_migration_never_overwrites_explicit_unblock() {
    let owner = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut core = logged_in_test_core("block-migrate", &owner, &Keys::generate());
    core.preferences.blocked_owner_pubkeys.push(target.clone());
    core.migrate_legacy_blocks();
    let legacy = core.private_block_event(&target).unwrap();
    assert_eq!(block_sync::block_event_version(&legacy).unwrap().0, 0);
    core.set_user_blocked(&target, false);
    assert!(core.apply_private_block_event(legacy));
    assert!(!core.is_owner_blocked(&target));
}

#[test]
fn private_block_history_preserves_old_group_context_but_stops_new_history() {
    let owner = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut core = logged_in_test_core("block-history", &owner, &Keys::generate());
    core.push_incoming_message_from("group:shared", Some("old".into()), "old context".into(), 20, None, None, Some(target.clone()), None);
    core.set_user_blocked(&target, true);
    assert_eq!(core.threads["group:shared"].messages.len(), 1);
    assert!(core.block_allows_history("group:shared", &target, 20));
    assert!(!core.block_allows_history("group:shared", &target, unix_now().get()));
    assert!(!core.block_allows_history(&target, &target, 20));
}

#[test]
fn private_block_offline_negentropy_sync_and_legacy_capability() {
    for supported in [false, true] {
        let owner = Keys::generate();
        let a = Keys::generate();
        let b = Keys::generate();
        let target = Keys::generate().public_key().to_hex();
        let mut left = logged_in_test_core("block-wire-left", &owner, &a);
        let mut right = logged_in_test_core("block-wire-right", &owner, &b);
        configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
        for roster in left.app_keys.values_mut() { roster.devices.sort_by(|a,b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex)); }
        right.app_keys = left.app_keys.clone();
        left.set_user_blocked(&target, true);
        let endpoint = Arc::new(left.runtime.block_on(fips_core::FipsEndpoint::builder().without_system_tun().bind()).unwrap());
        let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
        right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
        let mut request = serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1});
        if supported { request["privateEvents"] = 1.into(); }
        let request = serde_json::to_vec(&request).unwrap();
        left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
        right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
        let mut trace = Vec::new();
        for _ in 0..2048 {
            let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
            let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
            if !x && !y { break; }
        }
        assert_eq!(right.is_owner_blocked(&target), supported, "wire: {trace:?}");
        let sent_private = trace.iter().any(|value| value.to_string().contains("privateBlock"));
        assert_eq!(sent_private, supported);
        if supported {
            right.set_user_blocked(&target, false);
            left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
            right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
            for _ in 0..2048 {
                let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
                let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
                if !x && !y { break; }
            }
            assert!(!left.is_owner_blocked(&target), "offline unblock must reconcile");
            assert_eq!(left.private_block_event(&target), right.private_block_event(&target));
        }
        left.runtime.block_on(endpoint.shutdown()).unwrap();
    }
}

#[test]
fn private_block_drops_group_content_over_real_encrypted_transport() {
    let mut devices = sender_key_matrix_devices(2);
    let bob = devices[1].owner.public_key();
    let created = devices[0].engine.create_group("Shared".into(), vec![bob], unix_now()).unwrap();
    let group = created.snapshot.unwrap();
    deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
    for sender in 0..2 {
        let peer = devices[1-sender].owner.public_key();
        let sent = devices[sender].engine.send_direct_text(peer, "warmup", "hello", None, unix_now()).unwrap();
        deliver_protocol_effects_to_engine(&mut devices[1-sender].engine, &sent.effects);
    }
    let chat = group_chat_id(&group.group_id);
    let mut dirs = Vec::new();
    let cores = devices.into_iter().map(|device| {
        let (mut core, _, dir) = logged_in_test_core_with_updates("block-group", &device.owner, &device.device);
        dirs.push(dir);
        core.protocol_engine = Some(device.engine);
        core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group.clone()));
        core
    }).collect::<Vec<_>>();
    let mut cores: [AppCore;2] = cores.try_into().ok().unwrap();
    cores[1].send_message(&chat, "Before block", None);
    deliver_group_reaction_test_events(&mut cores);
    let target = cores[0].threads[&chat].messages.iter().find(|m| m.body == "Before block").unwrap().id.clone();
    cores[0].set_user_blocked(&bob.to_hex(), true);
    cores[1].toggle_reaction(&chat, &target, "❤");
    cores[1].send_typing(&chat);
    cores[1].send_message(&chat, "After block", None);
    deliver_group_reaction_test_events(&mut cores);
    assert!(!cores[0].threads[&chat].messages.iter().any(|m| m.body == "After block"));
    let old = cores[0].threads[&chat].messages.iter().find(|m| m.id == target).unwrap();
    assert_eq!(old.body, "Before block");
    assert!(old.reactions.is_empty());
    assert!(cores[0].typing_indicators.values().all(|value| value.chat_id != chat));
    assert!(cores[0].groups.contains_key(&group.group_id));
}

#[test]
fn private_block_current_sibling_can_forward_state_after_original_writer_removed() {
    let owner = Keys::generate();
    let original = Keys::generate();
    let current = Keys::generate();
    let fresh = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut writer = logged_in_test_core("block-original", &owner, &original);
    writer.set_user_blocked(&target, true);
    let event = writer.private_block_event(&target).unwrap();
    let mut receiver = logged_in_test_core("block-fresh", &owner, &fresh);
    configure_test_device_sync_profile(&mut receiver, &owner, &fresh, &current, None);
    let body = serde_json::json!({"type":"private-block-state","v":1,"event":event}).to_string();
    assert!(receiver.receive_block_control(owner.public_key(), Some(original.public_key()), &body));
    assert!(!receiver.is_owner_blocked(&target));
    assert!(receiver.receive_block_control(owner.public_key(), Some(current.public_key()), &body));
    assert!(receiver.is_owner_blocked(&target));
}

fn signed_block_transition(owner: &Keys, writer: &Keys, target: &str, revision: u64, at: u64, since: u64, blocked: bool) -> Event {
    EventBuilder::new(Kind::Custom(block_sync::BLOCK_CONTROL_KIND as u16), serde_json::json!({
        "v":1,"owner":owner.public_key().to_hex(),"target":target,"blocked":blocked,
        "revision":revision,"blockedSince":since,"deletedAt":since
    }).to_string()).tag(nostr::Tag::public_key(owner.public_key()))
        .tag(nostr::Tag::identifier(format!("iris:block:{target}")))
        .custom_created_at(Timestamp::from_secs(at)).allow_self_tagging().sign_with_keys(writer).unwrap()
}

#[test]
fn private_block_intervals_survive_multiple_cycles_late_unblock_and_device_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let (mut core, _, dir) = logged_in_test_core_with_updates("block-intervals", &owner, &device);
    let events = [
        signed_block_transition(&owner,&device,&target,1,100,100,true),
        signed_block_transition(&owner,&device,&target,2,200,100,false),
        signed_block_transition(&owner,&device,&target,3,300,300,true),
        signed_block_transition(&owner,&device,&target,4,400,300,false),
    ];
    // Live latest unblock alone attests its preceding block interval.
    assert!(core.apply_private_block_event(events[3].clone()));
    assert!(!core.block_allows_history("group:shared", &target, 350));
    assert!(!core.is_owner_blocked(&target));
    for index in [1,0,2,1] { assert!(core.apply_private_block_event(events[index].clone())); }
    for (created, allowed) in [(50,true),(100,false),(150,false),(200,true),(250,true),(300,false),(350,false),(400,true),(450,true)] {
        assert_eq!(core.block_allows_history("group:shared",&target,created),allowed,"at={created}");
    }
    assert!(!core.is_owner_blocked(&target), "stale transitions cannot override the latest unblock");
    let shared = core.app_store.shared();
    let json = storage::blocked_message_intervals(&shared.lock().unwrap(), &owner.public_key().to_hex()).unwrap();
    let intervals: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(intervals,serde_json::json!([
        {"author":target,"since":100,"until":200},
        {"author":target,"since":300,"until":400}
    ]));
    core.persist_best_effort_inner();
    drop(core);
    let mut restored = logged_in_test_core_at_data_dir(&owner,&Keys::generate(),dir.path().to_string_lossy().into_owned());
    restored.restore_device_sync_record_projection();
    for (created, allowed) in [(50,true),(150,false),(250,true),(350,false),(450,true)] {
        assert_eq!(restored.block_allows_history("group:shared",&target,created),allowed);
    }
    assert_eq!(restored.private_block_event(&target),Some(events[3].clone()));
}

#[test]
fn private_block_negentropy_repairs_intervals_before_lagging_sibling_history() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut left = logged_in_test_core("block-gap-left", &owner, &a);
    let mut right = logged_in_test_core("block-gap-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for roster in left.app_keys.values_mut() { roster.devices.sort_by(|a,b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex)); }
    right.app_keys = left.app_keys.clone();
    let base = unix_now().get()-1000;
    for (revision, at, since, blocked) in [(1,100,100,true),(2,200,100,false),(3,300,300,true),(4,400,300,false)] {
        assert!(left.apply_private_block_event(signed_block_transition(&owner,&a,&target,revision,base+at,base+since,blocked)));
    }
    // This sibling missed both blocked intervals and retained every group post.
    for offset in [50,150,250,350,450] {
        right.push_incoming_message_from("group:shared",Some(format!("at-{offset}")),"group post".into(),base+offset,None,None,Some(target.clone()),None);
    }
    right.persist_best_effort_inner();
    let endpoint = Arc::new(left.runtime.block_on(fips_core::FipsEndpoint::builder().without_system_tun().bind()).unwrap());
    let (left_tx,left_rx) = DeviceSyncTcpSender::test_channel(256,64*1024);
    let (right_tx,right_rx) = DeviceSyncTcpSender::test_channel(256,64*1024);
    left.install_device_sync_sender_for_test(endpoint.clone(),left_tx,vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(),right_tx,vec![test_fips_peer(&a)]);
    let request = serde_json::to_vec(&serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1,"privateEvents":1})).unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(),DEVICE_SYNC_PORT,&request);
    right.handle_device_sync_packet(&a.public_key().to_hex(),DEVICE_SYNC_PORT,&request);
    let mut trace = Vec::new();
    for _ in 0..2048 {
        let x = drain_history_wire(&mut left,&a,&mut right,&b,&left_rx,&mut trace);
        let y = drain_history_wire(&mut right,&b,&mut left,&a,&right_rx,&mut trace);
        if !x && !y { break; }
    }
    for offset in [50,150,250,350,450] {
        assert_eq!(has_device_sync_message(&left,"group:shared",&format!("at-{offset}")),![150,350].contains(&offset),"at={offset}; wire={trace:?}");
    }
    assert!(!left.is_owner_blocked(&target));
    assert!(!right.is_owner_blocked(&target));
    assert!(!right.block_allows_history("group:shared",&target,base+150));
    assert!(right.block_allows_history("group:shared",&target,base+250));
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}
