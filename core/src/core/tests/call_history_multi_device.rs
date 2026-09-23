#[test]
fn call_history_two_recipient_devices_record_answered_elsewhere_over_fips() {
    let caller_owner = Keys::generate();
    let caller_device = Keys::generate();
    let owner = Keys::generate();
    let first_device = Keys::generate();
    let second_device = Keys::generate();
    let mut peers = vec![
        logged_in_test_core_with_updates("history-caller", &caller_owner, &caller_device),
        logged_in_test_core_with_updates("history-first", &owner, &first_device),
        logged_in_test_core_with_updates("history-second", &owner, &second_device),
    ];
    let devices = [&caller_device, &first_device, &second_device];
    let inboxes: Vec<_> = peers
        .iter_mut()
        .map(|(core, _, _)| {
            call_test_peer(core, &caller_owner, &caller_device);
            call_test_peer(core, &owner, &first_device);
            core.app_keys
                .get_mut(&owner.public_key().to_hex())
                .unwrap()
                .devices
                .push(KnownAppKeyDevice {
                    identity_pubkey_hex: second_device.public_key().to_hex(),
                    created_at_secs: unix_now().get(),
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                });
            let (tx, rx) = flume::unbounded();
            core.core_sender = tx.clone();
            core.priority_sender = tx;
            rx
        })
        .collect();
    let addresses: Vec<_> = (0..3)
        .map(|_| {
            std::net::UdpSocket::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
        })
        .collect();
    for i in 0..3 {
        let remote = if i == 0 { 1 } else { 0 };
        peers[i].0.reconcile_calls_udp_for_test(
            addresses[i],
            addresses[remote],
            &test_fips_peer(devices[remote]).npub(),
        );
    }
    let endpoints: Vec<_> = peers
        .iter()
        .map(|(core, _, _)| core.device_sync_endpoint_for_test().unwrap())
        .collect();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !peers[0].0.runtime.block_on(device_sync_pair_is_connected([
        (&endpoints[0], &test_fips_peer(&first_device)),
        (&endpoints[0], &test_fips_peer(&second_device)),
    ])) {
        assert!(
            std::time::Instant::now() < deadline,
            "three loopback FIPS nodes did not connect"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let drop_sibling_end = std::cell::Cell::new(true);
    let wait = |peers: &mut Vec<(AppCore, flume::Receiver<AppUpdate>, tempfile::TempDir)>,
                predicate: &dyn Fn(&[&str]) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            for (index, ((core, _, _), inbox)) in peers.iter_mut().zip(&inboxes).enumerate() {
                for message in inbox.try_iter().take(128) {
                    if index == 2 && drop_sibling_end.get() {
                        if let CoreMsg::Internal(event) = &message {
                            if let InternalEvent::CallPacket { data, .. } = event.as_ref() {
                                if serde_json::from_slice::<serde_json::Value>(data)
                                    .ok()
                                    .is_some_and(|value| value["reason"] == "answered_elsewhere")
                                {
                                    drop_sibling_end.set(false);
                                    continue;
                                }
                            }
                        }
                    }
                    core.handle_message(message);
                }
            }
            let states: Vec<_> = peers
                .iter()
                .map(|(core, _, _)| core.state.call.as_ref().map_or("", |c| c.phase.as_str()))
                .collect();
            if predicate(&states) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "call phases: {:?}",
                states
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    peers[0].0.handle_action(AppAction::StartCall {
        chat_id: owner.public_key().to_hex(),
        video: true,
    });
    wait(&mut peers, &|states| {
        states[1] == "incoming" && states[2] == "incoming"
    });
    let id = peers[1].0.state.call.as_ref().unwrap().call_id.clone();
    peers[1].0.handle_action(AppAction::AnswerCallWithVoice {
        call_id: id.clone(),
    });
    peers[1].0.handle_action(AppAction::SetCallMediaConnected {
        call_id: id.clone(),
        connected: true,
    });
    wait(&mut peers, &|states| states[0] == "connected");
    // The unaccepted sibling must recover a lost disposition automatically.
    wait(&mut peers, &|states| states[2] == "ended");
    assert!(!drop_sibling_end.get());
    peers[0].0.handle_action(AppAction::SetCallMediaConnected {
        call_id: id.clone(),
        connected: true,
    });
    let caller = caller_owner.public_key().to_hex();
    let elsewhere = peers[2]
        .0
        .app_store
        .load_recent_messages(&caller, 10)
        .unwrap()[0]
        .call
        .clone()
        .unwrap();
    assert_eq!(elsewhere.outcome, "answered_elsewhere");
    assert_eq!(elsewhere.answered_at_secs, None);
    assert_eq!(elsewhere.duration_secs, 0);
    assert!(
        !elsewhere.video,
        "the other device answered the video offer with voice"
    );
    peers[0].0.handle_action(AppAction::EndCall { call_id: id });
    wait(&mut peers, &|states| states[1] == "ended");
    let answered = peers[1]
        .0
        .app_store
        .load_recent_messages(&caller, 10)
        .unwrap()[0]
        .call
        .clone()
        .unwrap();
    assert_eq!(answered.outcome, "answered");
    assert!(!answered.video);
    peers[0].0.handle_action(AppAction::StartCall {
        chat_id: owner.public_key().to_hex(),
        video: false,
    });
    wait(&mut peers, &|states| {
        states[1] == "incoming" && states[2] == "incoming"
    });
    let declined_id = peers[1].0.state.call.as_ref().unwrap().call_id.clone();
    peers[1].0.handle_action(AppAction::EndCall {
        call_id: declined_id,
    });
    wait(&mut peers, &|states| {
        states.iter().all(|state| *state == "ended")
    });
    for (core, _, _) in &peers {
        assert_eq!(
            core.state.call.as_ref().unwrap().end_reason.as_deref(),
            Some("Call declined")
        );
    }
    for (core, _, _) in &mut peers {
        core.shutdown();
    }
}
