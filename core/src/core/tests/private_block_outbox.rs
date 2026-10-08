#[test]
fn private_block_cancels_cloned_drain_and_preserves_other_peer_retry() {
    for start_worker in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let blocked = Keys::generate().public_key().to_hex();
        let other = Keys::generate().public_key().to_hex();
        let (mut core, _, _dir) = logged_in_test_core_with_updates("block-outbox", &owner, &device);
        // A current-thread runtime makes the not-yet-polled worker deterministic:
        // the full 16-event batch is cloned before block, then polled afterwards.
        core.runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let relay = crate::local_relay::TestRelay::start();
        relay
            .ignore_acknowledgements(MESSAGE_EVENT_KIND as u64)
            .unwrap();
        let (tx, rx) = flume::unbounded();
        core.core_sender = tx.clone();
        core.priority_sender = tx;
        let mut events = Vec::new();
        for index in 0..16 {
            let chat = if index % 2 == 0 { &blocked } else { &other };
            let event = EventBuilder::new(
                Kind::from(MESSAGE_EVENT_KIND as u16),
                format!("pending {index}"),
            )
            .sign_with_keys(&device)
            .unwrap();
            assert!(core.publish_runtime_event(
                event.clone(),
                "test",
                Some((format!("inner-{index}"), chat.clone()))
            ));
            events.push((event, chat.clone()));
        }
        core.logged_in.as_mut().unwrap().relay_urls = vec![RelayUrl::parse(relay.url()).unwrap()];
        core.retry_pending_relay_publishes("test_cloned_batch");
        assert_eq!(core.pending_relay_publish_inflight.len(), 16);
        let token = core.relay_transport_runtime.publish_drain_token;
        let cancelled = core
            .relay_transport_runtime
            .publish_drain_task
            .as_ref()
            .unwrap()
            .cancelled
            .clone();
        if start_worker {
            for _ in 0..400 {
                core.runtime.block_on(async {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                });
                if relay.events().len() == 4 {
                    break;
                }
            }
            assert_eq!(
                relay.events().len(),
                4,
                "four running sends must wait for acknowledgement"
            );
        }
        let already_delivered = relay.events();
        core.set_user_blocked(&blocked, true);
        assert!(cancelled.load(std::sync::atomic::Ordering::Acquire));
        assert_ne!(core.relay_transport_runtime.publish_drain_token, token);
        assert_eq!(core.pending_relay_publishes.len(), 8);
        assert!(core
            .pending_relay_publishes
            .values()
            .all(|pending| pending.chat_id.as_deref() == Some(&other)));
        let stored = core
            .app_store
            .load_pending_relay_publishes(&owner.public_key().to_hex())
            .unwrap();
        assert_eq!(
            stored.len(),
            8,
            "cancelled ciphertext must not survive restart"
        );
        assert!(stored
            .iter()
            .all(|pending| pending.chat_id.as_deref() == Some(&other)));
        // Late worker completion cannot acknowledge a cancelled message or advance
        // the new drain. Replayed protocol effects must also be rejected.
        let cancelled_event = events
            .iter()
            .find(|(_, chat)| chat == &blocked)
            .unwrap()
            .0
            .clone();
        core.handle_relay_publish_drain_finished(
            token,
            vec![RelayPublishDrainResult {
                event_id: cancelled_event.id.to_hex(),
                success: true,
                relay_urls: Vec::new(),
                detail: "late ack".into(),
            }],
        );
        assert!(!core.publish_protocol_event(ProtocolPublish {
            authored_at_secs: None,
            event: cancelled_event.clone(),
            chat_id: blocked.clone(),
            inner_event_id: Some("inner-0".into())
        }));
        core.set_user_blocked(&blocked, false);
        let healthy = crate::local_relay::TestRelay::start();
        core.logged_in.as_mut().unwrap().relay_urls = vec![RelayUrl::parse(healthy.url()).unwrap()];
        core.retry_pending_relay_publishes("unaffected_peer_retry");
        for _ in 0..200 {
            core.runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(5)).await;
            });
            for message in rx.try_iter() {
                if let CoreMsg::Internal(event) = message {
                    core.handle_internal(*event);
                }
            }
            if core.pending_relay_publishes.is_empty() {
                break;
            }
        }
        assert!(
            core.pending_relay_publishes.is_empty(),
            "other conversation retries successfully"
        );
        assert_eq!(
            relay.events(),
            already_delivered,
            "cancelled worker must not send the rest of its cloned batch"
        );
        let published = healthy.events();
        for (event, chat) in events {
            assert_eq!(
                published
                    .iter()
                    .any(|value| value["id"] == event.id.to_hex()),
                chat == other
            );
        }
    }
}

#[test]
fn private_block_removes_unprepared_text_and_never_replays_it_after_unblock() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let target = peer.public_key().to_hex();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("block-unprepared", &owner, &device);
    core.send_message(&target, "still waiting for discovery", None);
    assert!(core.has_queued_direct_text_messages());
    core.set_user_blocked(&target, true);
    assert!(!core.threads.contains_key(&target));
    core.set_user_blocked(&target, false);
    let engine = core.protocol_engine.as_mut().unwrap();
    observe_peer_appkeys_for_test(engine, &peer, &[peer_device.public_key()], 1);
    observe_peer_device_invite_for_test(engine, &peer, &peer_device, 2);
    assert!(!core.drain_queued_direct_text_messages("test_after_unblock"));
    assert!(!core
        .pending_relay_publishes
        .values()
        .any(|pending| pending.chat_id.as_deref() == Some(&target)));
}
#[test]
fn private_block_late_unblock_state_cancels_old_intents_but_keeps_reopened_sends() {
    for block_first in [true, false] {
        let owner = Keys::generate();
        let device = Keys::generate();
        // Force BLOCK before UNBLOCK in durable event-ID order as well as in
        // the explicit live catch-up case, making the restart regression exact.
        let (peer, block, unblock) = loop {
            let peer = Keys::generate();
            let target = peer.public_key().to_hex();
            let block = signed_block_transition(&owner, &device, &target, 1, 100, 100, true);
            let unblock = signed_block_transition(&owner, &device, &target, 2, 200, 100, false);
            if block.id < unblock.id {
                break (peer, block, unblock);
            }
        };
        let ready = Keys::generate();
        let late = Keys::generate();
        let target = peer.public_key().to_hex();
        let (mut core, _, dir) =
            logged_in_test_core_with_updates("block-late-unblock-queue", &owner, &device);
        core.app_store.bind_account(owner.public_key()).unwrap();
        core.preferences.nostr_relay_urls.clear();
        core.preferences.nearby_enabled = false;
        let engine = core.protocol_engine.as_mut().unwrap();
        observe_current_device_appkeys_for_test(engine, &owner, &device);
        observe_peer_device_invite_for_test(engine, &peer, &ready, 2);
        observe_peer_appkeys_for_test(engine, &peer, &[ready.public_key(), late.public_key()], 3);
        for (body, at) in [
            ("old pending text", 50),
            ("during block prepared", 150),
            ("new reopened text", 250),
        ] {
            core.send_direct_message(&target, body, UnixSeconds(at), None);
        }
        let engine = core.protocol_engine.take();
        core.send_direct_message(&target, "during block unprepared", UnixSeconds(175), None);
        core.send_direct_message(&target, "new reopened unprepared", UnixSeconds(275), None);
        core.protocol_engine = engine;
        let messages = core.threads[&target].messages.clone();
        let new = messages
            .iter()
            .find(|message| message.created_at_secs == 250)
            .unwrap()
            .clone();
        assert!(!new.delivery_trace.outer_event_ids.is_empty());
        assert!(
            core.pending_relay_publishes
                .values()
                .all(|pending| pending.created_at_secs > 200),
            "sealed wrapper times are deliberately newer than all block cutoffs"
        );
        let transitions = if block_first {
            [block, unblock]
        } else {
            [unblock, block]
        };
        assert!(core.apply_private_block_event(transitions[0].clone()));
        if block_first {
            assert!(!core.drain_queued_direct_text_messages("held_open_block"));
            let engine = core.protocol_engine.as_mut().unwrap();
            assert!(
                engine
                    .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
                    .unwrap()
                    .effects
                    .is_empty(),
                "held newer intents must not be consumed by protocol retry"
            );
            assert!(
                core.pending_relay_publish_batch_event_ids(16).0.is_empty(),
                "held wrappers must not reach the message server"
            );
        }
        assert!(core.apply_private_block_event(transitions[1].clone()));
        assert!(!core.is_owner_blocked(&target));
        let stored = core
            .app_store
            .load_pending_relay_publishes(&owner.public_key().to_hex())
            .unwrap();
        for message in messages
            .iter()
            .filter(|message| message.created_at_secs < 200)
        {
            for id in &message.delivery_trace.outer_event_ids {
                assert!(!core.pending_relay_publishes.contains_key(id));
                assert!(!stored.iter().any(|pending| &pending.event_id == id));
            }
        }
        for id in &new.delivery_trace.outer_event_ids {
            assert!(core.pending_relay_publishes.contains_key(id));
            assert!(stored.iter().any(|pending| &pending.event_id == id));
        }
        assert!(
            core.threads[&target]
                .messages
                .iter()
                .all(|message| message.created_at_secs > 200),
            "blocked-period messages are removed from the direct conversation"
        );
        assert!(core.drain_queued_direct_text_messages("after_unblock"));
        assert!(core.threads[&target]
            .messages
            .iter()
            .filter(|message| message.created_at_secs > 200)
            .all(|message| !message.delivery_trace.outer_event_ids.is_empty()));
        core.persist_best_effort_inner();
        drop(core);

        let (tx, _rx) = flume::unbounded();
        let mut restored = AppCore::new(
            flume::unbounded().0,
            tx,
            dir.path().to_string_lossy().into_owned(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        restored
            .start_session(
                owner.public_key(),
                Some(owner.clone()),
                device.clone(),
                true,
                true,
            )
            .unwrap();
        assert!(!restored.is_owner_blocked(&target));
        assert!(
            restored.threads[&target]
                .messages
                .iter()
                .all(|message| message.created_at_secs > 200),
            "blocked-period messages cannot return after a full app restart"
        );
        let stored = restored
            .app_store
            .load_pending_relay_publishes(&owner.public_key().to_hex())
            .unwrap();
        assert!(
            new.delivery_trace
                .outer_event_ids
                .iter()
                .all(|id| stored.iter().any(|pending| &pending.event_id == id)),
            "replaying historical BLOCK first during startup must preserve later ciphertext"
        );
        let engine = restored.protocol_engine.as_mut().unwrap();
        assert_eq!(
            engine
                .retire_pending_direct_sends(peer.public_key(), Some(199))
                .unwrap(),
            0
        );
        assert_eq!(
            engine
                .retire_pending_direct_sends(peer.public_key(), None)
                .unwrap(),
            2,
            "both post-unblock remote-device intents survive complete app restoration"
        );
    }
}
#[test]
fn private_block_catch_up_removes_during_only_direct_conversation() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("block-during-only", &owner, &device);
    core.send_direct_message(&peer, "queued during block", UnixSeconds(150), None);
    assert!(core.has_queued_direct_text_messages());
    for (revision, at, blocked) in [(1, 100, true), (2, 200, false)] {
        assert!(core.apply_private_block_event(signed_block_transition(
            &owner, &device, &peer, revision, at, 100, blocked
        )));
    }
    assert!(!core.threads.contains_key(&peer));
    assert!(core
        .app_store
        .load_recent_messages(&peer, 50)
        .unwrap()
        .is_empty());
    assert!(!core.drain_queued_direct_text_messages("after_catch_up"));
    assert!(core.pending_relay_publishes.is_empty());
}
