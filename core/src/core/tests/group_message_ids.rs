#[test]
fn group_message_ids_are_distinct_across_linked_devices_at_the_same_clock_tick() {
    let owner = Keys::generate();
    let first = logged_in_test_core("group-message-id-first", &owner, &Keys::generate());
    let second = logged_in_test_core("group-message-id-second", &owner, &Keys::generate());
    let authored_at = unix_now();
    let millis = authored_at.get() * 1000;
    let mut ids = HashSet::new();
    for core in [&first, &second, &first] {
        let mut rumor = core
            .prepare_group_event(
                "same-clock-tick",
                CHAT_MESSAGE_KIND,
                "Repeated",
                Vec::new(),
                pairwise_codec::EncodeOptions::new(authored_at.get(), millis),
            )
            .unwrap();
        assert_eq!(rumor.pubkey, owner.public_key());
        assert_eq!(rumor.created_at.as_secs(), authored_at.get());
        assert_eq!(rumor.content, "Repeated");
        assert!(
            ids.insert(rumor.id().to_hex()),
            "separate sends must stay distinct even without saving prior messages"
        );
    }
}

#[test]
fn group_message_ids_keep_identical_sends_distinct_on_delivery_and_restart() {
    let mut devices = sender_key_matrix_devices(2);
    let recipient = devices[1].owner.public_key();
    let created = devices[0]
        .engine
        .create_group("Repeated messages".into(), vec![recipient], unix_now())
        .unwrap();
    let group = created.snapshot.unwrap();
    deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
    let chat_id = group_chat_id(&group.group_id);
    let mut dirs = Vec::new();
    let cores = devices
        .into_iter()
        .map(|device| {
            let (mut core, _, dir) = logged_in_test_core_with_updates(
                "group-message-ids",
                &device.owner,
                &device.device,
            );
            dirs.push(dir);
            core.protocol_engine = Some(device.engine);
            core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group.clone()));
            core
        })
        .collect::<Vec<_>>();
    let mut cores: [AppCore; 2] = cores.try_into().ok().unwrap();
    let authored_at = unix_now();
    let expiration = Some(authored_at.get() + 3600);
    for _ in 0..2 {
        cores[0].send_group_message(&chat_id, "Same message", authored_at, expiration);
    }
    let sent_ids = cores[0].threads[&chat_id]
        .messages
        .iter()
        .filter(|message| message.body == "Same message")
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(sent_ids.len(), 2);
    assert_ne!(
        sent_ids[0], sent_ids[1],
        "each intentional send needs a distinct ID"
    );

    deliver_group_reaction_test_events(&mut cores);
    let message_id = &sent_ids[0];
    let millis = authored_at.get() * 1000;
    let mut previous_ids = cores[0]
        .pending_relay_publishes
        .values()
        .filter_map(|pending| pending.inner_event_id.clone())
        .collect::<HashSet<_>>();
    for emoji in ["👍", "", "👍"] {
        cores[0].send_group_event(
            &chat_id,
            REACTION_KIND,
            emoji,
            vec![vec!["e".to_string(), message_id.clone()]],
            Some(millis),
        );
        let current_ids = cores[0]
            .pending_relay_publishes
            .values()
            .filter_map(|pending| pending.inner_event_id.clone())
            .collect::<HashSet<_>>();
        assert_eq!(
            current_ids.difference(&previous_ids).count(),
            1,
            "repeated controls at the exact same millisecond need fresh IDs"
        );
        previous_ids = current_ids;
        deliver_group_reaction_test_events(&mut cores);
        let received = cores[1].threads[&chat_id]
            .messages
            .iter()
            .find(|message| message.id == *message_id)
            .unwrap();
        assert_eq!(
            received
                .reactors
                .first()
                .map(|reactor| reactor.emoji.as_str()),
            (!emoji.is_empty()).then_some(emoji),
            "add, remove, and re-add must all reach the other group member"
        );
    }
    for core in &mut cores {
        let messages = core.threads[&chat_id]
            .messages
            .iter()
            .filter(|message| message.body == "Same message")
            .collect::<Vec<_>>();
        assert_eq!(
            messages.len(),
            2,
            "both messages must survive encrypted delivery"
        );
        for message in messages {
            assert!(sent_ids.contains(&message.id));
            assert_eq!(message.created_at_secs, authored_at.get());
            assert_eq!(message.expires_at_secs, expiration);
        }
        core.persist_best_effort();
    }
    let identities = cores.each_ref().map(|core| {
        let account = core.logged_in.as_ref().unwrap();
        (
            account.owner_keys.clone().unwrap(),
            account.device_keys.clone(),
        )
    });
    drop(cores);
    for ((owner, device), dir) in identities.into_iter().zip(dirs) {
        let core = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            dir.path().to_string_lossy().into_owned(),
        );
        let stored = core.app_store.load_thread(&chat_id, 80).unwrap().unwrap();
        let messages = stored
            .messages
            .iter()
            .filter(|message| message.body == "Same message")
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 2, "both messages must survive restart");
        assert!(messages
            .iter()
            .all(|message| sent_ids.contains(&message.id)));
    }
}
