#[test]
fn native_private_control_interop_fixture() {
    let mut pair = chat_read_receipt_pair("private-control-wire-fixture");
    let owner = pair.owner.public_key();
    let contact = Keys::generate().public_key().to_hex();
    assert!(pair.a.edit_private_contact_fields(
        &contact,
        private_contact_patch(
            serde_json::json!({"favorite":true,"nickname":"Fixture friend","note":"Fixture note"}),
        )
    ));
    pair.a
        .set_current_device_labels("Fixture phone", "Iris Chat");
    pair.a.handle_action(AppAction::SetChatMuted {
        chat_id: contact.clone(),
        muted: true,
    });
    pair.a.handle_action(AppAction::SetChatPinned {
        chat_id: contact.clone(),
        pinned: true,
    });

    let mut cases = BTreeMap::new();
    for event in pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND) {
        let delivery = pair
            .b
            .protocol_engine
            .as_mut()
            .unwrap()
            .process_direct_message_event(&event)
            .unwrap()
            .unwrap();
        let rumor: UnsignedEvent = serde_json::from_str(&delivery.content).unwrap();
        rumor.verify_id().unwrap();
        let kind = rumor.kind.as_u16();
        if !matches!(kind, 10449 | 10450 | 10452 | 10453) {
            continue;
        }
        assert_eq!(delivery.sender, owner);
        assert_eq!(delivery.sender_device, Some(pair.a_device.public_key()));
        assert_eq!(delivery.conversation_owner, Some(owner));
        assert_eq!(
            rumor
                .tags
                .iter()
                .filter(|tag| { tag.as_slice().first().is_some_and(|name| name == "p") })
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>(),
            vec![vec!["p".to_string(), owner.to_hex()]],
            "real native producer must retain the recipient expected by web apps"
        );

        // Use the exact previous builder behavior, including its ID calculation.
        // Do not manually remove tags from an already identified event.
        let mut legacy = EventBuilder::new(rumor.kind, rumor.content.clone())
            .tags(rumor.tags.iter().cloned())
            .custom_created_at(rumor.created_at)
            .build(owner);
        legacy.ensure_id();
        legacy.verify_id().unwrap();
        assert!(legacy
            .tags
            .iter()
            .all(|tag| { tag.as_slice().first().is_none_or(|name| name != "p") }));
        for candidate in [&rumor, &legacy] {
            let content = serde_json::to_string(candidate).unwrap();
            assert_eq!(
                pair.b.private_sibling_control_disposition(
                    owner,
                    Some(pair.a_device.public_key()),
                    &parse_runtime_rumor(&content).unwrap(),
                ),
                None,
                "current and previously emitted native forms are authorized"
            );
        }
        cases.insert(
            kind,
            serde_json::json!({"kind":kind,"event":rumor,"legacyEvent":legacy}),
        );
    }
    assert_eq!(
        cases.keys().copied().collect::<Vec<_>>(),
        vec![10449, 10450, 10452, 10453]
    );
    let fixture = serde_json::json!({
        "version":1, "owner":owner.to_hex(), "senderDevice":pair.a_device.public_key().to_hex(),
        "recipientDevice":pair.b_device.public_key().to_hex(), "contact":contact,
        "cases":cases.into_values().collect::<Vec<_>>()
    });
    if let Some(path) = std::env::var_os("IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
    }
}
