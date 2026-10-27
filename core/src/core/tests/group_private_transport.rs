struct SenderKeyMatrixDevice {
    owner: Keys,
    device: Keys,
    engine: ProtocolEngine,
}

#[test]
fn network_group_intermediates_require_authenticated_ratchet_delivery() {
    use base64::Engine;

    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    farm.send(0, &chat_id, "only through the private carrier");
    let (header_event, _) = private_group_wake_events(&farm, &group_id);
    let mut legacy_bytes = 7_u32.to_be_bytes().to_vec();
    legacy_bytes.extend_from_slice(&0_u32.to_be_bytes());
    legacy_bytes.extend_from_slice(b"unwrapped group ciphertext");
    let legacy_event = EventBuilder::new(
        Kind::from(MESSAGE_EVENT_KIND as u16),
        base64::engine::general_purpose::STANDARD.encode(legacy_bytes),
    )
    .tag(nostr::Tag::parse(["v", "2"]).unwrap())
    .sign_with_keys(&Keys::generate())
    .unwrap();
    assert!(parse_group_sender_key_message_event(&legacy_event).is_ok());
    assert!(parse_group_sender_key_message_event_unchecked(&header_event).is_ok());

    let mut changed_roster = farm.devices[0].core.groups[&group_id].clone();
    changed_roster.name = "Only authenticated roster delivery".into();
    changed_roster.revision += 1;
    changed_roster.updated_at = NdrUnixSeconds(changed_roster.updated_at.get() + 1);
    let roster = group_roster_unsigned_event(farm.owners[0].public_key(), &changed_roster)
        .unwrap()
        .sign_with_keys(&farm.owners[0])
        .unwrap();
    assert!(nostr_double_ratchet::parse_group_roster_fact_event(&roster).is_ok());

    let receiver = &mut farm.devices[1].core;
    let before = receiver.protocol_engine.as_ref().unwrap().debug_snapshot();
    let groups_before = serde_json::to_value(&receiver.groups).unwrap();
    let messages_before = receiver.threads[&chat_id].messages.len();
    let publishes_before = receiver.pending_relay_publishes.len();
    for channel in ["message servers", "nearby"] {
        for event in [&header_event, &legacy_event, &roster] {
            let wire = serde_json::to_string(event).unwrap();
            let decoded = serde_json::from_str(&wire).unwrap();
            assert!(receiver.handle_relay_event_with_channel(decoded, channel));
        }
    }
    let after = receiver.protocol_engine.as_ref().unwrap().debug_snapshot();
    assert_eq!(
        after.pending_group_sender_key_message_count,
        before.pending_group_sender_key_message_count
    );
    assert_eq!(
        after.pending_group_sender_key_repair_count,
        before.pending_group_sender_key_repair_count
    );
    assert_eq!(
        after.pending_group_fanout_count,
        before.pending_group_fanout_count
    );
    assert_eq!(receiver.pending_relay_publishes.len(), publishes_before);
    assert_eq!(receiver.threads[&chat_id].messages.len(), messages_before);
    assert_eq!(
        serde_json::to_value(&receiver.groups).unwrap(),
        groups_before
    );

    // The same nested ciphertext remains usable through its recipient's
    // authenticated ratchet envelope, without exposing the group author.
    farm.pump();
    assert!(farm.devices[1].core.threads[&chat_id]
        .messages
        .iter()
        .any(|message| message.body == "only through the private carrier"));

    // The local signed-record API used by authenticated sibling replay remains
    // available; a valid roster signature alone did not admit it from the wire.
    let applied = farm.devices[1]
        .core
        .protocol_engine
        .as_mut()
        .unwrap()
        .ingest_group_roster_fact_event(&roster)
        .unwrap()
        .unwrap();
    assert_eq!(applied.snapshot.unwrap().name, changed_roster.name);
}

impl SenderKeyMatrixDevice {
    fn new() -> Self {
        let owner = Keys::generate();
        let device = Keys::generate();
        let engine = test_protocol_engine(&owner, &device);
        Self {
            owner,
            device,
            engine,
        }
    }
}

fn sender_key_matrix_devices(count: usize) -> Vec<SenderKeyMatrixDevice> {
    let mut devices = (0..count)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    observe_sender_key_matrix_protocol_state(&mut devices);
    devices
}

fn observe_sender_key_matrix_protocol_state(devices: &mut [SenderKeyMatrixDevice]) {
    let identities = devices
        .iter()
        .map(|device| {
            (
                device.owner.clone(),
                device.device.clone(),
                device.engine.local_invite().expect("local invite"),
            )
        })
        .collect::<Vec<_>>();
    for recipient in devices.iter_mut() {
        for (owner, device, invite) in &identities {
            if recipient.device.public_key() != device.public_key() {
                observe_local_invite_for_test(&mut recipient.engine, owner, device, invite);
            } else {
                observe_current_device_appkeys_for_test(&mut recipient.engine, owner, device);
            }
        }
    }
}

fn observe_local_invite_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
    invite: &Invite,
) {
    observe_peer_appkeys_for_test(
        engine,
        owner,
        &[device.public_key()],
        invite.created_at.get(),
    );
    let mut invite = invite.clone();
    invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let event = nostr_double_ratchet::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(device)
        .expect("signed invite");
    engine
        .observe_invite_event(&event)
        .expect("observe peer local invite");
}

fn ordered_protocol_events(effects: &[ProtocolEffect]) -> Vec<Event> {
    protocol_publish_events(effects)
        .into_iter()
        .cloned()
        .collect()
}

fn is_non_target_direct_message_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("Invalid header")
        || message.contains("invalid header")
        || message.contains("Failed to decrypt header with available keys")
        || message.contains("invalid HMAC")
}

fn test_event_has_tag(event: &Event, name: &str) -> bool {
    event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some(name))
}

fn is_group_sender_key_outer_candidate_for_test(engine: &ProtocolEngine, event: &Event) -> bool {
    let has_header = test_event_has_tag(event, "header");
    if has_header && !engine.is_known_group_sender_event_author(event.pubkey) {
        return false;
    }
    if has_header {
        parse_group_sender_key_message_event_unchecked(event).is_ok()
    } else {
        parse_group_sender_key_message_event(event).is_ok()
    }
}

fn sender_key_outer_events_for_engine<'a>(
    engine: &ProtocolEngine,
    effects: &'a [ProtocolEffect],
    event_ids: &[String],
) -> Vec<&'a Event> {
    protocol_payload_events_for_result(effects, event_ids)
        .into_iter()
        .filter(|event| is_group_sender_key_outer_candidate_for_test(engine, event))
        .collect()
}

fn sender_key_outer_count(
    engine: &ProtocolEngine,
    effects: &[ProtocolEffect],
    event_ids: &[String],
) -> usize {
    sender_key_outer_events_for_engine(engine, effects, event_ids).len()
}

// Decrypt exactly one recipient's ratchet carrier while retaining its nested
// group ciphertext for the lower-level ordering/replay tests below.
fn take_private_group_cipher_for_test(
    recipient: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Event {
    let local_device = recipient
        .session_manager_snapshot()
        .local_device_pubkey
        .to_hex();
    for event in ordered_protocol_events(effects) {
        if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
            recipient
                .observe_invite_response_event(&event)
                .expect("invite response");
            continue;
        }
        if event.kind.as_u16() as u32 != MESSAGE_EVENT_KIND
            || !event
                .tags
                .public_keys()
                .any(|key| key.to_hex() == local_device)
        {
            continue;
        }
        let Some(decrypted) = recipient
            .process_direct_message_event(&event)
            .expect("decrypt recipient's private group carrier")
        else {
            continue;
        };
        let Some(rumor) = parse_runtime_rumor(&decrypted.content) else {
            continue;
        };
        if rumor.kind == private_group_wake::GROUP_WAKE_KIND {
            let nested =
                private_group_wake::decode(&rumor.content).expect("nested group ciphertext");
            assert!(
                private_group_wake::sender_matches(recipient, decrypted.sender, &nested),
                "group ciphertext must belong to its authenticated ratchet sender"
            );
            return nested;
        }
        recipient
            .process_group_pairwise_payload(
                decrypted.content.as_bytes(),
                decrypted.sender,
                decrypted.sender_device,
            )
            .expect("apply preceding group controls");
    }
    panic!("recipient has no private group delivery")
}

// Sender-key ordering tests inspect only the local intermediate ciphertext. A
// disposable recipient receives the private transport and all preceding controls;
// the actual recipient keeps its deliberately missing metadata/distributions.
fn private_group_cipher_from_sender_for_test(
    sender: &ProtocolEngine,
    recipient: &ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Event {
    private_group_cipher_with_controls_for_test(sender, recipient, &[], effects)
}

fn private_group_cipher_with_controls_for_test(
    sender: &ProtocolEngine,
    recipient: &ProtocolEngine,
    controls: &[ProtocolEffect],
    effects: &[ProtocolEffect],
) -> Event {
    let mut probe = recipient.fork_for_test().expect("fork recipient state");
    let selected = protocol_publish_events(effects)
        .into_iter()
        .map(|event| event.id)
        .collect::<HashSet<_>>();
    let mut pending = controls.to_vec();
    pending.extend(sender.pending_group_publish_effects());
    deliver_invite_response_effects_to_engine(&mut probe, &pending);
    let preceding = pending
        .into_iter()
        .filter(|effect| {
            let ProtocolEffect::Publish(publish) = effect;
            !selected.contains(&publish.event.id)
                && publish.event.kind.as_u16() as u32 != INVITE_RESPONSE_KIND
        })
        .collect::<Vec<_>>();
    deliver_protocol_effects_to_engine(&mut probe, &preceding);
    let nested = take_private_group_cipher_for_test(&mut probe, effects);
    assert!(sender
        .group_sender_event_pubkeys_for_owner(
            PublicKey::from_slice(
                &sender
                    .session_manager_snapshot()
                    .local_owner_pubkey
                    .to_bytes()
            )
            .expect("sender owner")
        )
        .contains(&nested.pubkey));
    nested
}

fn apply_protocol_event_to_engine(
    engine: &mut ProtocolEngine,
    event: &Event,
    group_events: &mut Vec<GroupIncomingEvent>,
) {
    if event.kind.as_u16() as u32 == INVITE_EVENT_KIND {
        let retry = engine
            .observe_invite_event(event)
            .expect("observe invite event");
        group_events.extend(retry.group_result.events);
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.effects),
            group_events,
        );
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.group_result.effects),
            group_events,
        );
        return;
    }

    if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
        let retry = engine
            .observe_invite_response_event(event)
            .expect("observe invite response event");
        group_events.extend(retry.group_result.events);
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.effects),
            group_events,
        );
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.group_result.effects),
            group_events,
        );
        return;
    }

    if is_group_sender_key_outer_candidate_for_test(engine, event) {
        let result = engine
            .process_group_outer_event(event)
            .expect("process sender-key outer event");
        group_events.extend(result.events);
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&result.effects),
            group_events,
        );
        return;
    }

    if parse_message_event(event).is_ok() {
        let decrypted = match engine.process_direct_message_event(event) {
            Ok(decrypted) => decrypted,
            Err(error) if is_non_target_direct_message_error(&error) => None,
            Err(error) => panic!("process pairwise protocol event: {error}"),
        };
        if let Some(decrypted) = decrypted {
            if let Some(rumor) = parse_runtime_rumor(&decrypted.content) {
                if rumor.kind == private_group_wake::GROUP_WAKE_KIND {
                    if let Some(nested) = private_group_wake::decode(&rumor.content) {
                        if private_group_wake::sender_matches(engine, decrypted.sender, &nested) {
                            let result = engine
                                .process_group_outer_event(&nested)
                                .expect("process authenticated private group delivery");
                            group_events.extend(result.events);
                            apply_protocol_events_to_engine(
                                engine,
                                &ordered_protocol_events(&result.effects),
                                group_events,
                            );
                        }
                    }
                    return;
                }
            }
            let result = engine
                .process_group_pairwise_payload(
                    decrypted.content.as_bytes(),
                    decrypted.sender,
                    decrypted.sender_device,
                )
                .expect("process group pairwise payload");
            group_events.extend(result.events);
            apply_protocol_events_to_engine(
                engine,
                &ordered_protocol_events(&result.effects),
                group_events,
            );
        }
    }
}

fn apply_protocol_events_to_engine(
    engine: &mut ProtocolEngine,
    events: &[Event],
    group_events: &mut Vec<GroupIncomingEvent>,
) {
    for event in events {
        apply_protocol_event_to_engine(engine, event, group_events);
    }
}

fn deliver_protocol_effects_to_engine(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Vec<GroupIncomingEvent> {
    let mut group_events = Vec::new();
    apply_protocol_events_to_engine(engine, &ordered_protocol_events(effects), &mut group_events);
    group_events
}

fn deliver_invite_response_effects_to_engine(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Vec<GroupIncomingEvent> {
    let mut group_events = Vec::new();
    for event in ordered_protocol_events(effects) {
        if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
            apply_protocol_event_to_engine(engine, &event, &mut group_events);
        }
    }
    group_events
}

fn apply_protocol_event_to_engine_once(
    engine: &mut ProtocolEngine,
    event: &Event,
) -> (Vec<GroupIncomingEvent>, Vec<ProtocolEffect>) {
    if event.kind.as_u16() as u32 == INVITE_EVENT_KIND {
        let retry = engine
            .observe_invite_event(event)
            .expect("observe invite event");
        let mut effects = retry.effects;
        effects.extend(retry.group_result.effects);
        return (retry.group_result.events, effects);
    }

    if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
        let retry = engine
            .observe_invite_response_event(event)
            .expect("observe invite response event");
        let mut effects = retry.effects;
        effects.extend(retry.group_result.effects);
        return (retry.group_result.events, effects);
    }

    if is_group_sender_key_outer_candidate_for_test(engine, event) {
        let result = engine
            .process_group_outer_event(event)
            .expect("process sender-key outer event");
        return (result.events, result.effects);
    }

    if parse_message_event(event).is_ok() {
        let decrypted = match engine.process_direct_message_event(event) {
            Ok(decrypted) => decrypted,
            Err(error) if is_non_target_direct_message_error(&error) => None,
            Err(error) => panic!("process pairwise protocol event: {error}"),
        };
        if let Some(decrypted) = decrypted {
            if let Some(rumor) = parse_runtime_rumor(&decrypted.content) {
                if rumor.kind == private_group_wake::GROUP_WAKE_KIND {
                    if let Some(nested) = private_group_wake::decode(&rumor.content) {
                        if private_group_wake::sender_matches(engine, decrypted.sender, &nested) {
                            let result = engine
                                .process_group_outer_event(&nested)
                                .expect("process authenticated private group delivery");
                            return (result.events, result.effects);
                        }
                    }
                    return (Vec::new(), Vec::new());
                }
            }
            let result = engine
                .process_group_pairwise_payload(
                    decrypted.content.as_bytes(),
                    decrypted.sender,
                    decrypted.sender_device,
                )
                .expect("process group pairwise payload");
            return (result.events, result.effects);
        }
    }

    (Vec::new(), Vec::new())
}

fn deliver_protocol_effects_to_engine_once(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> (Vec<GroupIncomingEvent>, Vec<ProtocolEffect>) {
    let mut group_events = Vec::new();
    let mut followup_effects = Vec::new();
    for event in ordered_protocol_events(effects) {
        let (events, effects) = apply_protocol_event_to_engine_once(engine, &event);
        group_events.extend(events);
        followup_effects.extend(effects);
    }
    (group_events, followup_effects)
}

fn deliver_protocol_effects_without_feedback(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Vec<GroupIncomingEvent> {
    let local_device = engine
        .session_manager_snapshot()
        .local_device_pubkey
        .to_hex();
    let mut group_events = Vec::new();
    for event in ordered_protocol_events(effects) {
        if event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND {
            let direct_target = event.tags.iter().find_map(|tag| {
                let values = tag.as_slice();
                (values.first().map(|value| value.as_str()) == Some("p"))
                    .then(|| values.get(1).map(|value| value.as_str()))
                    .flatten()
            });
            if direct_target.is_some_and(|target| target != local_device) {
                continue;
            }
        }
        let (events, _) = apply_protocol_event_to_engine_once(engine, &event);
        group_events.extend(events);
    }
    group_events
}

fn group_events_contain_body(
    events: &[GroupIncomingEvent],
    group_id: &str,
    sender_owner: PublicKey,
    sender_device: PublicKey,
    body: &[u8],
) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            GroupIncomingEvent::Message(message)
                if message.group_id == group_id
                    && message.sender_owner == ndr_owner_pubkey(sender_owner)
                    && message.sender_device == Some(ndr_device_pubkey(sender_device))
                    && message.body == body
        )
    })
}
