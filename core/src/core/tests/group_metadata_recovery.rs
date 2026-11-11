use super::*;

fn forget_group_context_for_test(farm: &mut GroupScaleFarm, receiver: usize) {
    let owner = &farm.owners[farm.devices[receiver].owner];
    let entry = &mut farm.devices[receiver];
    let mut saved = runtime_state_json(&entry.core, owner, &entry.keys);
    saved["group_manager"]["groups"] = serde_json::json!([]);
    saved["group_manager"]["sender_keys"] = serde_json::json!([]);
    saved["group_roster_fact_histories"] = serde_json::json!({});
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        entry.core.app_store.shared(),
        owner.public_key().to_hex(),
        entry.keys.public_key().to_hex(),
    ));
    storage
        .put(TEST_PROTOCOL_ENGINE_STATE_KEY, saved.to_string())
        .unwrap();
    entry.core.protocol_engine = Some(
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &entry.keys)
            .unwrap(),
    );
    entry.core.groups.clear();
    entry.core.threads.retain(|id, _| !is_group_chat_id(id));
    entry
        .core
        .app_store
        .shared()
        .lock()
        .unwrap()
        .execute("DELETE FROM groups", [])
        .unwrap();
    entry.core.persist_best_effort();
    // The fixture created the group moments ago; model a later cold start,
    // after any initial out-of-order metadata request cooldown has elapsed.
    for entry in &farm.devices {
        entry
            .core
            .app_store
            .shared()
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_meta SET value=json_array(0,json_extract(value,'$[1]'))
             WHERE key LIKE 'group-context-recovery:%'",
                [],
            )
            .unwrap();
    }
    for entry in &mut farm.devices {
        let owner = &farm.owners[entry.owner];
        let mut saved = runtime_state_json(&entry.core, owner, &entry.keys);
        for answered in saved["answered_group_sender_key_repairs"]
            .as_array_mut()
            .unwrap()
        {
            answered["next_response_at_secs"] = serde_json::json!(0);
        }
        let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
            entry.core.app_store.shared(),
            owner.public_key().to_hex(),
            entry.keys.public_key().to_hex(),
        ));
        storage
            .put(TEST_PROTOCOL_ENGINE_STATE_KEY, saved.to_string())
            .unwrap();
        entry.core.protocol_engine = Some(
            ProtocolEngine::load_or_create_for_local_device(
                storage,
                owner.public_key(),
                &entry.keys,
            )
            .unwrap(),
        );
    }
}

#[test]
fn missing_group_context_recovers_from_admin_and_member_without_empty_chats() {
    for sender in [0, 2] {
        let (mut farm, group_id, chat_id) = private_group_wake_farm();
        forget_group_context_for_test(&mut farm, 1);
        let start = farm.history.len();
        let body = "recover this group's missing details automatically";
        farm.send(sender, &chat_id, body);
        farm.pump();
        let receiver = &farm.devices[1].core;
        assert!(receiver.groups.contains_key(&group_id), "sender={sender}");
        assert!(receiver.threads[&chat_id]
            .messages
            .iter()
            .any(|message| message.body == body));
        for entry in &farm.devices {
            for owner in &farm.owners {
                if owner.public_key() != farm.owners[entry.owner].public_key() {
                    assert!(
                        !entry
                            .core
                            .threads
                            .contains_key(&owner.public_key().to_hex()),
                        "group connections must not create empty direct chats"
                    );
                }
            }
        }
        for event in &farm.history[start..] {
            let wire = serde_json::to_string(event).unwrap();
            assert!(!wire.contains(&group_id));
            for owner in &farm.owners {
                assert!(!wire.contains(&owner.public_key().to_hex()));
            }
            assert!(matches!(
                event.kind.as_u16() as u32,
                MESSAGE_EVENT_KIND | INVITE_RESPONSE_KIND
            ));
            // Only an invite response names a target: the public invite key.
            assert_eq!(
                event.tags.public_keys().count(),
                usize::from(event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
            );
        }
        for entry in &farm.devices {
            for filter in build_protocol_subscription_filters(
                &entry.core.compute_protocol_subscription_plan().unwrap(),
            ) {
                let wire = serde_json::to_string(&filter).unwrap();
                assert!(!wire.contains(&group_id));
            }
        }
    }
}

fn restart_group_recovery_device(farm: &mut GroupScaleFarm, index: usize) {
    let GroupScaleDevice {
        owner,
        keys,
        core,
        _dir,
        delivered,
        online,
    } = farm.devices.remove(index);
    drop(core);
    let account = farm.owners[owner].clone();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        _dir.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_session(
        account.public_key(),
        Some(account),
        keys.clone(),
        true,
        true,
    )
    .unwrap();
    farm.devices.insert(
        index,
        GroupScaleDevice {
            owner,
            keys,
            core,
            _dir,
            delivered,
            online,
        },
    );
}

#[test]
fn group_context_recovery_survives_restart_and_waits_for_admin_without_flooding() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    forget_group_context_for_test(&mut farm, 1);
    farm.devices[0].online = false;
    let body = "metadata recovery while the admin is offline";
    farm.send(2, &chat_id, body);
    farm.pump();
    assert!(
        !farm.devices[1].core.groups.contains_key(&group_id),
        "an ordinary member cannot install an unauthenticated membership list"
    );
    farm.devices[1].core.persist_app_state().unwrap();
    restart_group_recovery_device(&mut farm, 1);
    let before = farm.history.len();
    for _ in 0..5 {
        farm.devices[1]
            .core
            .retry_protocol_engine_pending_work("group-recovery-restart-test");
        farm.pump();
    }
    assert_eq!(
        farm.history.len(),
        before,
        "restart must preserve the request cooldown"
    );
    assert!(!farm.devices[1].core.groups.contains_key(&group_id));
    farm.devices[0].online = true;
    farm.pump();
    farm.retry();
    assert!(farm.devices[1].core.groups.contains_key(&group_id));
    assert!(farm.devices[1].core.threads[&chat_id]
        .messages
        .iter()
        .any(|message| message.body == body));
}

#[test]
fn group_context_requests_reject_nonmembers_forwarding_loops_and_wrong_groups() {
    let (mut farm, group_id, _) = private_group_wake_farm();
    let member = farm.owners[1].public_key();
    let stranger = Keys::generate().public_key();
    let author = farm.devices[0]
        .core
        .protocol_engine
        .as_ref()
        .unwrap()
        .group_sender_event_pubkeys_for_owner(farm.owners[0].public_key())[0];
    for (receiver, sender, request) in [
        (0, stranger, serde_json::json!({"v":1,"group_id":group_id})),
        (
            0,
            member,
            serde_json::json!({"v":1,"group_id":group_id,"requester":stranger}),
        ),
        (
            2,
            member,
            serde_json::json!({"v":1,"group_id":group_id,"requester":farm.owners[0].public_key()}),
        ),
        (
            0,
            member,
            serde_json::json!({"v":1,"group_id":"another-group","author":author}),
        ),
        (0, member, serde_json::json!({"v":2,"group_id":group_id})),
    ] {
        let core = &mut farm.devices[receiver].core;
        let pending = core.pending_relay_publishes.len();
        assert!(core.receive_group_context_request(sender, &request.to_string()));
        assert_eq!(core.pending_relay_publishes.len(), pending);
    }
    let core = &mut farm.devices[0].core;
    core.preferences.blocked_owner_pubkeys.push(member.to_hex());
    let pending = core.pending_relay_publishes.len();
    assert!(core.receive_group_context_request(
        member,
        &serde_json::json!({"v":1,"group_id":group_id}).to_string()
    ));
    assert_eq!(core.pending_relay_publishes.len(), pending);
}

#[test]
fn group_context_request_push_is_silent_and_read_only() {
    let (mut farm, group_id, _) = private_group_wake_farm();
    let admin = farm.owners[0].clone();
    let member = farm.owners[1].public_key();
    farm.devices[0]
        .core
        .preferences
        .accepted_owner_pubkeys
        .push(member.to_hex());
    farm.devices[0].core.persist_app_state().unwrap();
    let request = UnsignedEvent::new(
        member,
        Timestamp::now(),
        Kind::Custom(crate::core::group_metadata_recovery::GROUP_CONTEXT_REQUEST_KIND as u16),
        [],
        serde_json::json!({"v":1,"group_id":group_id}).to_string(),
    );
    let sent = farm.devices[1]
        .core
        .protocol_engine
        .as_mut()
        .unwrap()
        .send_direct_unsigned_event_to_peer_only(
            admin.public_key(),
            &admin.public_key().to_hex(),
            request,
            unix_now(),
        )
        .unwrap();
    let event = protocol_publish_events(&sent.effects)
        .into_iter()
        .find(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap();
    let entry = &farm.devices[0];
    let before = runtime_state_json(&entry.core, &admin, &entry.keys);
    let resolution = decrypt_mobile_push_notification(
        entry._dir.path().to_string_lossy().into_owned(),
        admin.public_key().to_hex(),
        entry.keys.secret_key().to_secret_hex(),
        serde_json::json!({"event":event}).to_string(),
    );
    assert!(!resolution.should_show);
    assert_eq!(runtime_state_json(&entry.core, &admin, &entry.keys), before);
}

#[test]
fn queued_group_distribution_recovers_metadata_after_restart_without_another_message() {
    use nostr_double_ratchet::{
        GroupPairwiseCommand, GroupPayloadCodec, GroupPayloadEncodeContext, JsonGroupPayloadCodecV1,
    };
    let (mut farm, group_id, _) = private_group_wake_farm();
    forget_group_context_for_test(&mut farm, 1);
    let sender = farm.owners[0].public_key();
    let device = farm.devices[0].keys.public_key();
    let distribution = latest_sender_key_distribution_for_test(
        farm.devices[0].core.protocol_engine.as_ref().unwrap(),
        &group_id,
        NdrUnixSeconds(unix_now().get()),
    );
    let payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device_pubkey(device),
                created_at: NdrUnixSeconds(unix_now().get()),
            },
            &GroupPairwiseCommand::SenderKeyDistribution { distribution },
        )
        .unwrap();
    // Simulate a crash after the engine durably queues the distribution, before
    // the app can notice the missing group or send a recovery request.
    let engine = farm.devices[1].core.protocol_engine.as_mut().unwrap();
    assert!(
        engine
            .process_group_pairwise_payload(&payload, sender, Some(device))
            .unwrap()
            .consumed
    );
    assert_eq!(engine.missing_group_metadata_sources().len(), 1);
    restart_group_recovery_device(&mut farm, 1);
    farm.devices[1]
        .core
        .retry_protocol_engine_pending_work("group-context-after-crash");
    farm.pump();
    assert!(farm.devices[1].core.groups.contains_key(&group_id));
    assert!(farm.devices[1]
        .core
        .protocol_engine
        .as_ref()
        .unwrap()
        .missing_group_metadata_sources()
        .is_empty());
}

#[test]
fn expired_group_context_request_does_not_generate_a_response() {
    let (mut farm, group_id, _) = private_group_wake_farm();
    let member = farm.owners[1].public_key();
    let request = UnsignedEvent::new(
        member,
        Timestamp::from(1),
        Kind::Custom(crate::core::group_metadata_recovery::GROUP_CONTEXT_REQUEST_KIND as u16),
        [nostr::Tag::expiration(Timestamp::from(2))],
        serde_json::json!({"v":1,"group_id":group_id}).to_string(),
    );
    let rumor = parse_runtime_rumor(&serde_json::to_string(&request).unwrap()).unwrap();
    let device = farm.devices[1].keys.public_key();
    let core = &mut farm.devices[0].core;
    let before = core.pending_relay_publishes.len();
    assert_eq!(
        core.receive_private_runtime_control(member, Some(device), &rumor),
        Some(true)
    );
    assert_eq!(core.pending_relay_publishes.len(), before);
}

#[test]
fn direct_group_id_message_requests_metadata_before_showing_the_chat() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    forget_group_context_for_test(&mut farm, 1);
    let receiver = farm.owners[1].public_key();
    let body = "recover from just the group ID";
    let rumor = UnsignedEvent::new(
        farm.owners[2].public_key(),
        Timestamp::now(),
        Kind::Custom(CHAT_MESSAGE_KIND as u16),
        [nostr::Tag::parse(["l", group_id.as_str()]).unwrap()],
        body,
    );
    let sent = farm.devices[2]
        .core
        .protocol_engine
        .as_mut()
        .unwrap()
        .send_direct_unsigned_event_to_peer_only(receiver, &chat_id, rumor, unix_now())
        .unwrap();
    for event in protocol_publish_events(&sent.effects) {
        farm.devices[1].core.handle_relay_event(event.clone());
    }
    assert!(!farm.devices[1].core.threads.contains_key(&chat_id));
    farm.pump();
    farm.retry();
    assert!(farm.devices[1].core.groups.contains_key(&group_id));
    assert_eq!(
        farm.devices[1].core.threads[&chat_id]
            .messages
            .iter()
            .filter(|message| message.body == body)
            .count(),
        1
    );
}
