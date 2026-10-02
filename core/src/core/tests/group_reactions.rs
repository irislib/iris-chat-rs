fn deliver_group_reaction_test_events(cores: &mut [AppCore; 2]) {
    let mut delivered = HashSet::new();
    for _ in 0..10 {
        let mut progressed = false;
        for sender in 0..2 {
            let mut events = cores[sender]
                .pending_relay_publishes
                .values()
                .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
                .collect::<Vec<_>>();
            events.sort_by_key(|event| (event.created_at, event.id));
            for event in events {
                if delivered.insert((sender, event.id)) {
                    cores[1 - sender].handle_relay_event(event);
                    progressed = true;
                }
            }
        }
        if !progressed {
            return;
        }
    }
    panic!("group reaction delivery did not settle");
}

#[test]
fn group_reactions_add_replace_and_remove_across_encrypted_transport() {
    let mut devices = sender_key_matrix_devices(2);
    let bob = devices[1].owner.public_key();
    let created = devices[0]
        .engine
        .create_group("Reactions".into(), vec![bob], unix_now())
        .unwrap();
    let group = created.snapshot.unwrap();
    deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
    // Establish both directions as real group members do through their invites.
    for sender in 0..2 {
        let peer = devices[1 - sender].owner.public_key();
        let sent = devices[sender]
            .engine
            .send_direct_text(peer, "reaction-warmup", "hello", None, unix_now())
            .unwrap();
        deliver_protocol_effects_to_engine(&mut devices[1 - sender].engine, &sent.effects);
    }
    let chat_id = group_chat_id(&group.group_id);
    let mut dirs = Vec::new();
    let cores = devices
        .into_iter()
        .map(|device| {
            let (mut core, _, dir) =
                logged_in_test_core_with_updates("group-reactions", &device.owner, &device.device);
            dirs.push(dir);
            core.protocol_engine = Some(device.engine);
            core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group.clone()));
            core
        })
        .collect::<Vec<_>>();
    let mut cores: [AppCore; 2] = cores.try_into().ok().unwrap();
    cores[0].send_message(&chat_id, "React to this message", None);
    deliver_group_reaction_test_events(&mut cores);
    let message_id = cores[1].threads[&chat_id]
        .messages
        .iter()
        .find(|message| message.body == "React to this message")
        .expect("the other group member received the encrypted message")
        .id
        .clone();
    let message_counts = cores
        .each_ref()
        .map(|core| core.threads[&chat_id].messages.len());
    for (emoji, expected) in [
        ("👍", Some("👍")),
        ("❤️", Some("❤️")),
        ("❤️", None),
        ("👍", Some("👍")),
    ] {
        cores[1].toggle_reaction(&chat_id, &message_id, emoji);
        let local = cores[1].threads[&chat_id]
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .unwrap();
        assert_eq!(
            local.reactors.first().map(|reactor| reactor.emoji.as_str()),
            expected,
            "the local reaction must update before any network delivery"
        );
        deliver_group_reaction_test_events(&mut cores);
        for (index, core) in cores.iter().enumerate() {
            let thread = &core.threads[&chat_id];
            assert_eq!(thread.messages.len(), message_counts[index]);
            let message = thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .unwrap();
            assert_eq!(
                message
                    .reactors
                    .iter()
                    .find(|reactor| reactor.author == bob.to_hex())
                    .map(|reactor| reactor.emoji.as_str()),
                expected,
                "group member {index} should see the reaction update"
            );
            if expected.is_some() {
                assert_eq!(message.reactions.len(), 1);
                assert_eq!(message.reactions[0].count, 1);
                assert_eq!(message.reactions[0].reacted_by_me, index == 1);
            } else {
                assert!(message.reactions.is_empty());
            }
        }
    }
}
