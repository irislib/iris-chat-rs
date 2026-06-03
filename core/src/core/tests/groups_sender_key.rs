struct SenderKeyMatrixDevice {
    owner: Keys,
    device: Keys,
    engine: ProtocolEngine,
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
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(
                device.public_key(),
                invite.created_at.get(),
            )]),
            invite.created_at.get(),
        )
        .expect("peer appkeys");
    let mut invite = invite.clone();
    invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
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
            Err(error) if is_non_target_direct_message_error(&error) => {
                None
            }
            Err(error) => panic!("process pairwise protocol event: {error}"),
        };
        if let Some(decrypted) = decrypted {
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
    if is_group_sender_key_outer_candidate_for_test(engine, event) {
        let result = engine
            .process_group_outer_event(event)
            .expect("process sender-key outer event");
        return (result.events, result.effects);
    }

    if parse_message_event(event).is_ok() {
        let decrypted = match engine.process_direct_message_event(event) {
            Ok(decrypted) => decrypted,
            Err(error) if is_non_target_direct_message_error(&error) => {
                None
            }
            Err(error) => panic!("process pairwise protocol event: {error}"),
        };
        if let Some(decrypted) = decrypted {
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

#[test]
fn appcore_create_group_defaults_to_sender_key_protocol() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let result = engine
        .create_group("sender-key group".to_string(), Vec::new(), UnixSeconds(3))
        .expect("create sender-key group");
    let group = result.snapshot.expect("created group snapshot");

    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(
        engine.group_manager_snapshot_for_test().sender_keys.len(),
        1,
        "sender-key group creation should seed a local sender-key record"
    );
    assert_eq!(
        engine.known_group_sender_event_pubkeys().len(),
        1,
        "sender-key group creation should make its sender event author subscribable"
    );
}

#[test]
fn appcore_sender_key_group_send_publishes_one_outer_event() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let created = engine
        .create_group("sender-key group".to_string(), Vec::new(), UnixSeconds(3))
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");

    let result = engine
        .send_group_payload(
            &group.group_id,
            b"sender-key message".to_vec(),
            Some("inner-message-id".to_string()),
        )
        .expect("send sender-key group payload");

    assert_eq!(result.event_ids.len(), 1);
    let message_publish = result
        .effects
        .iter()
        .find_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if result.event_ids.contains(&publish.event.id.to_string()) =>
            {
                Some(publish)
            }
            _ => None,
        })
        .expect("sender-key message publish");
    assert_eq!(
        message_publish.inner_event_id.as_deref(),
        Some("inner-message-id")
    );
    let outer_events = sender_key_outer_events_for_engine(&engine, &result.effects, &result.event_ids);

    assert_eq!(
        outer_events.len(),
        1,
        "sender-key group send should publish one shared outer event"
    );
    assert_eq!(outer_events[0].id.to_string(), result.event_ids[0]);
    assert_eq!(
        parse_group_sender_key_message_event_unchecked(outer_events[0])
            .expect("sender-key outer event")
            .sender_event_pubkey
            .to_bytes(),
        outer_events[0].pubkey.to_bytes()
    );
}

#[test]
fn appcore_sender_key_outer_before_distribution_retries_after_control_state() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut alice = test_protocol_engine(&alice_owner, &alice_device);
    let mut bob = test_protocol_engine(&bob_owner, &bob_device);
    observe_current_device_appkeys_for_test(&mut alice, &alice_owner, &alice_device);
    observe_current_device_appkeys_for_test(&mut bob, &bob_owner, &bob_device);
    observe_peer_device_invite_for_test(&mut alice, &bob_owner, &bob_device, 50);
    observe_peer_device_invite_for_test(&mut bob, &alice_owner, &alice_device, 50);

    let created = alice
        .create_group(
            "sender-key group".to_string(),
            vec![bob_owner.public_key()],
            UnixSeconds(51),
        )
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");
    let group_id = group.group_id.clone();
    let sender_key = alice
        .group_manager_snapshot_for_test()
        .sender_keys
        .into_iter()
        .find(|record| record.group_id == group_id)
        .expect("local sender-key record");
    let key_id = sender_key.latest_key_id.expect("latest sender key id");
    let state = sender_key
        .states
        .iter()
        .find(|state| state.key_id() == key_id)
        .expect("sender-key state");
    let distribution = nostr_double_ratchet::SenderKeyDistribution {
        group_id: group.group_id.clone(),
        key_id,
        sender_event_pubkey: sender_key.sender_event_pubkey,
        chain_key: state.chain_key(),
        iteration: state.iteration(),
        created_at: NdrUnixSeconds(51),
    };

    let sent = alice
        .send_group_payload(
            &group.group_id,
            b"queued until sender-key distribution".to_vec(),
            Some("sender-key-inner".to_string()),
        )
        .expect("send sender-key group payload");
    let outer = sender_key_outer_events_for_engine(&alice, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();

    let pending = bob
        .process_group_outer_event(&outer)
        .expect("process outer before distribution");
    assert!(pending.consumed);
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        1
    );

    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device.public_key()),
            created_at: NdrUnixSeconds(52),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot {
            snapshot: group.clone(),
        },
    )
    .expect("metadata payload");
    let distribution_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device.public_key()),
            created_at: NdrUnixSeconds(53),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyDistribution { distribution },
    )
    .expect("sender-key distribution payload");

    let metadata_result = bob
        .process_group_pairwise_payload(
            &metadata_payload,
            alice_owner.public_key(),
            Some(alice_device.public_key()),
        )
        .expect("process metadata");
    let distribution_result = bob
        .process_group_pairwise_payload(
            &distribution_payload,
            alice_owner.public_key(),
            Some(alice_device.public_key()),
        )
        .expect("process distribution");
    assert!(matches!(
        metadata_result.events.as_slice(),
        [GroupIncomingEvent::MetadataUpdated(_)]
    ));
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        0
    );
    assert!(distribution_result.events.iter().any(|event| matches!(
        event,
        GroupIncomingEvent::Message(message)
            if message.group_id == group_id
                && message.sender_owner == ndr_owner_pubkey(alice_owner.public_key())
                && message.sender_device == Some(ndr_device_pubkey(alice_device.public_key()))
                && message.body == b"queued until sender-key distribution".to_vec()
    )));
    let retry = bob
        .retry_pending_protocol(NdrUnixSeconds(54))
        .expect("retry after pending sender-key outer already applied");
    assert!(
        retry.group_result.events.is_empty(),
        "applied pending sender-key outer must not replay on later retry"
    );
}

#[test]
fn appcore_sender_key_missing_rotated_distribution_repairs_and_applies_pending_outer() {
    let mut devices = sender_key_matrix_devices(3);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let bob_owner = devices[bob].owner.public_key();
    let carol_owner = devices[carol].owner.public_key();
    let alice_owner = devices[alice].owner.public_key();
    let alice_device = devices[alice].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key repair".to_string(),
            vec![bob_owner, carol_owner],
            UnixSeconds(200),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &created.effects);

    let removed = devices[alice]
        .engine
        .remove_group_member(&group_id, carol_owner)
        .expect("remove carol and rotate sender key");
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &removed.effects);

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"after missed rotation".to_vec(),
            Some("sender-key-repair-inner".to_string()),
        )
        .expect("send with rotated sender key");
    let outer = sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();

    let pending = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer missing rotated key");
    assert!(pending.pending);
    assert!(
        devices[bob]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count
            >= 1
    );
    assert_eq!(
        devices[bob]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_repair_count,
        1
    );
    assert!(
        !pending.effects.is_empty(),
        "missing sender-key distribution should emit a repair request"
    );

    let (alice_events, repair_response_effects) =
        deliver_protocol_effects_to_engine_once(&mut devices[alice].engine, &pending.effects);
    assert!(
        alice_events.is_empty(),
        "repair request should not be surfaced as an app group event"
    );
    assert!(
        !repair_response_effects.is_empty(),
        "sender should answer repair request with pairwise key material"
    );

    let (bob_repaired_events, revision_request_effects) = deliver_protocol_effects_to_engine_once(
        &mut devices[bob].engine,
        &repair_response_effects,
    );
    let bob_final_events = if group_events_contain_body(
        &bob_repaired_events,
        &group_id,
        alice_owner,
        alice_device,
        b"after missed rotation",
    ) {
        bob_repaired_events
    } else {
        assert!(
            !revision_request_effects.is_empty(),
            "decrypting with repaired key should request missing metadata revision if the repaired key does not apply immediately"
        );
        let (_alice_events, metadata_response_effects) = deliver_protocol_effects_to_engine_once(
            &mut devices[alice].engine,
            &revision_request_effects,
        );
        assert!(
            !metadata_response_effects.is_empty(),
            "sender should answer revision repair request with current metadata"
        );
        let (bob_final_events, _followup) = deliver_protocol_effects_to_engine_once(
            &mut devices[bob].engine,
            &metadata_response_effects,
        );
        bob_final_events
    };
    assert!(
        group_events_contain_body(
            &bob_final_events,
            &group_id,
            alice_owner,
            alice_device,
            b"after missed rotation"
        ),
        "pending sender-key outer should apply after key and metadata repair"
    );
    assert_eq!(
        devices[bob]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_repair_count,
        0
    );
}

#[test]
fn appcore_sender_key_repair_response_survives_sender_restart() {
    let alice_storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut devices = (0..3)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    devices[0].engine = test_protocol_engine_with_storage(
        &devices[0].owner,
        &devices[0].device,
        alice_storage.clone(),
    );
    observe_sender_key_matrix_protocol_state(&mut devices);

    let alice = 0;
    let bob = 1;
    let carol = 2;
    let alice_owner = devices[alice].owner.clone();
    let alice_device = devices[alice].device.clone();
    let alice_owner_pubkey = alice_owner.public_key();
    let alice_device_pubkey = alice_device.public_key();
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key sender restart repair".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(340),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &created.effects);

    let removed = devices[alice]
        .engine
        .remove_group_member(&group_id, carol_owner_pubkey)
        .expect("remove carol and rotate sender key");
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &removed.effects);

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"repair after sender restart".to_vec(),
            Some("sender-key-sender-restart-inner".to_string()),
        )
        .expect("send with rotated sender key");
    let outer = sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();
    let pending = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer missing rotated key");
    assert!(!pending.effects.is_empty());

    devices[alice].engine =
        test_protocol_engine_with_storage(&alice_owner, &alice_device, alice_storage.clone());
    let (_alice_events, key_response_effects) =
        deliver_protocol_effects_to_engine_once(&mut devices[alice].engine, &pending.effects);
    assert!(
        !key_response_effects.is_empty(),
        "restarted sender should answer repair from distribution history"
    );
    let (bob_after_key_events, revision_request_effects) =
        deliver_protocol_effects_to_engine_once(&mut devices[bob].engine, &key_response_effects);
    if group_events_contain_body(
        &bob_after_key_events,
        &group_id,
        alice_owner_pubkey,
        alice_device_pubkey,
        b"repair after sender restart",
    ) {
        return;
    }
    assert!(
        !revision_request_effects.is_empty(),
        "key repair should apply immediately or request missing metadata"
    );
    let (_alice_events, metadata_response_effects) = deliver_protocol_effects_to_engine_once(
        &mut devices[alice].engine,
        &revision_request_effects,
    );
    let (bob_final_events, _followup) = deliver_protocol_effects_to_engine_once(
        &mut devices[bob].engine,
        &metadata_response_effects,
    );
    assert!(
        group_events_contain_body(
            &bob_final_events,
            &group_id,
            alice_owner_pubkey,
            alice_device_pubkey,
            b"repair after sender restart"
        ),
        "pending sender-key outer should apply after restarted sender answers repair"
    );
}

#[test]
fn appcore_sender_key_duplicate_replay_idempotent() {
    let mut devices = sender_key_matrix_devices(2);
    let alice = 0;
    let bob = 1;
    let bob_owner = devices[bob].owner.public_key();
    let alice_owner = devices[alice].owner.public_key();
    let alice_device = devices[alice].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key duplicate replay".to_string(),
            vec![bob_owner],
            UnixSeconds(360),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"dedupe sender-key replay".to_vec(),
            Some("sender-key-dedupe-inner".to_string()),
        )
        .expect("send sender-key payload");
    let first = deliver_protocol_effects_to_engine(&mut devices[bob].engine, &sent.effects);
    assert!(group_events_contain_body(
        &first,
        &group_id,
        alice_owner,
        alice_device,
        b"dedupe sender-key replay"
    ));

    let duplicate = deliver_protocol_effects_to_engine(&mut devices[bob].engine, &sent.effects);
    assert!(
        !group_events_contain_body(
            &duplicate,
            &group_id,
            alice_owner,
            alice_device,
            b"dedupe sender-key replay"
        ),
        "duplicate relay replay must not emit a duplicate app message"
    );
}

#[test]
fn appcore_sender_key_removed_member_repair_denied() {
    let mut devices = sender_key_matrix_devices(3);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let bob_device_pubkey = devices[bob].device.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key repair denied".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(380),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &created.effects);

    let removed = devices[alice]
        .engine
        .remove_group_member(&group_id, bob_owner_pubkey)
        .expect("remove bob and rotate sender key");
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &removed.effects);
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &removed.effects);

    let distribution =
        latest_sender_key_distribution_for_test(&devices[alice].engine, &group_id, NdrUnixSeconds(381));
    let request = nostr_double_ratchet::SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: distribution.sender_event_pubkey,
        key_id: Some(distribution.key_id),
        message_number: Some(0),
        required_revision: None,
        created_at: NdrUnixSeconds(381),
    };
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let repair_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(bob_device_pubkey),
            created_at: NdrUnixSeconds(381),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyRepairRequest { request },
    )
    .expect("repair request payload");

    let response = devices[alice]
        .engine
        .process_group_pairwise_payload(&repair_payload, bob_owner_pubkey, Some(bob_device_pubkey))
        .expect("process removed member repair request");
    assert!(
        response.effects.is_empty(),
        "removed member repair request must not leak sender-key material"
    );
    assert!(response.consumed);
}

#[test]
fn appcore_sender_key_mixed_order_storm_converges() {
    let mut devices = sender_key_matrix_devices(3);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let bob_owner = devices[bob].owner.public_key();
    let carol_owner = devices[carol].owner.public_key();
    let alice_owner = devices[alice].owner.public_key();
    let alice_device = devices[alice].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key mixed order".to_string(),
            vec![bob_owner, carol_owner],
            UnixSeconds(400),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &created.effects);

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"mixed order first".to_vec(),
            Some("sender-key-mixed-order-inner".to_string()),
        )
        .expect("send before bob has control state");
    let outer = sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();

    let first_outer = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer before control state");
    assert!(first_outer.consumed);
    assert_eq!(
        devices[bob]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        1
    );
    let duplicate_outer = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process duplicate pending outer");
    assert!(duplicate_outer.consumed);

    let mut bob_events = Vec::new();
    apply_protocol_events_to_engine(
        &mut devices[bob].engine,
        &ordered_protocol_events(&created.effects),
        &mut bob_events,
    );

    let applied_count = bob_events
        .iter()
        .filter(|event| {
            matches!(
                event,
                GroupIncomingEvent::Message(message)
                    if message.group_id == group_id
                        && message.sender_owner == ndr_owner_pubkey(alice_owner)
                        && message.sender_device == Some(ndr_device_pubkey(alice_device))
                        && message.body == b"mixed order first".to_vec()
            )
        })
        .count();
    assert_eq!(
        applied_count, 1,
        "mixed-order control and duplicate outer replay should apply exactly once"
    );
}

#[test]
fn appcore_sender_key_group_create_prepares_pairwise_metadata_and_distribution() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 10);

    let result = engine
        .create_group(
            "sender-key group".to_string(),
            vec![peer_owner.public_key()],
            UnixSeconds(20),
        )
        .expect("create sender-key group");
    let group = result.snapshot.expect("created group snapshot");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(
        result.event_ids.len(),
        2,
        "sender-key group creation should send metadata and sender-key distribution over pairwise control"
    );
    assert_eq!(payload_events.len(), 2);
    assert!(
        payload_events
            .iter()
            .all(|event| parse_message_event(event).is_ok()
                && parse_group_sender_key_message_event(event).is_err()),
        "sender-key group creation must not publish a group outer message before app payloads"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &peer_owner.public_key().to_hex()),
        2,
        "the peer should receive both metadata and sender-key distribution control messages"
    );
}

#[test]
fn appcore_sender_key_add_member_sends_current_distribution_pairwise() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let created = engine
        .create_group("sender-key group".to_string(), Vec::new(), UnixSeconds(20))
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");
    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 21);

    let result = engine
        .add_group_members(&group.group_id, vec![peer_owner.public_key()])
        .expect("add sender-key group member");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(
        result.event_ids.len(),
        2,
        "adding a member should send metadata and the current sender-key distribution"
    );
    assert!(
        payload_events
            .iter()
            .all(|event| parse_message_event(event).is_ok()
                && parse_group_sender_key_message_event(event).is_err()),
        "add-member control traffic should remain pairwise"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &peer_owner.public_key().to_hex()),
        2
    );
}

#[test]
fn appcore_sender_key_remove_member_rotates_key_only_to_remaining_members() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let carol_owner = Keys::generate();
    let carol_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_device_invite_for_test(&mut engine, &bob_owner, &bob_device, 30);
    observe_peer_device_invite_for_test(&mut engine, &carol_owner, &carol_device, 31);

    let created = engine
        .create_group(
            "sender-key group".to_string(),
            vec![bob_owner.public_key(), carol_owner.public_key()],
            UnixSeconds(32),
        )
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");

    let result = engine
        .remove_group_member(&group.group_id, carol_owner.public_key())
        .expect("remove sender-key group member");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(
        result.event_ids.len(),
        3,
        "removal should send metadata to removed member and metadata plus rotated sender key to remaining member"
    );
    assert!(
        payload_events
            .iter()
            .all(|event| parse_message_event(event).is_ok()
                && parse_group_sender_key_message_event(event).is_err()),
        "remove-member control traffic should remain pairwise"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &bob_owner.public_key().to_hex()),
        3,
        "removal should publish metadata/control events for affected members"
    );
}

#[test]
fn appcore_legacy_pairwise_group_metadata_is_ignored() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 40);

    let group_id = "legacy-pairwise-group".to_string();
    let mut snapshot = test_group_snapshot(
        &group_id,
        "Legacy Pairwise Group",
        owner.public_key(),
        vec![owner.public_key(), peer_owner.public_key()],
        vec![owner.public_key()],
        1,
    );
    snapshot.protocol = nostr_double_ratchet::GroupProtocol::pairwise_fanout_v1();
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(device.public_key()),
            created_at: NdrUnixSeconds(41),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("metadata payload");
    let outcome = engine
        .process_group_pairwise_payload(
            &metadata_payload,
            owner.public_key(),
            Some(device.public_key()),
        )
        .expect("consume legacy pairwise group metadata");
    assert!(outcome.consumed);
    assert!(outcome.events.is_empty());
    assert!(outcome.effects.is_empty());

    let error = engine
        .send_group_payload(
            &group_id,
            b"legacy pairwise body".to_vec(),
            Some("legacy-inner".to_string()),
        )
        .expect_err("ignored legacy metadata must not install an outgoing group");
    assert!(
        error.to_string().contains("unknown group")
            || error.to_string().contains("unsupported legacy group protocol"),
        "unexpected error: {error}"
    );
}

#[test]
fn appcore_sender_key_four_member_matrix_delivers_one_outer_per_sender() {
    let mut devices = sender_key_matrix_devices(4);
    let member_pubkeys = devices
        .iter()
        .skip(1)
        .map(|device| device.owner.public_key())
        .collect::<Vec<_>>();
    let created = devices[0]
        .engine
        .create_group(
            "sender-key matrix".to_string(),
            member_pubkeys,
            UnixSeconds(100),
        )
        .expect("create sender-key matrix group");
    let group_id = created.snapshot.expect("created group").group_id;
    let create_effects = created.effects.clone();

    for recipient in devices.iter_mut().skip(1) {
        deliver_protocol_effects_to_engine(&mut recipient.engine, &create_effects);
    }

    for sender_index in 0..devices.len() {
        for recipient_index in 0..devices.len() {
            if recipient_index == sender_index {
                continue;
            }
            let recipient_owner = devices[recipient_index].owner.public_key();
            let warmup = devices[sender_index]
                .engine
                .send_direct_text(
                    recipient_owner,
                    "sender-key-matrix-warmup",
                    "warmup",
                    None,
                    UnixSeconds(90 + sender_index as u64),
                )
                .expect("warm sender-key matrix pairwise session");
            deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &warmup.effects);
        }
    }

    for sender_index in 0..devices.len() {
        let sender_owner = devices[sender_index].owner.public_key();
        let sender_device = devices[sender_index].device.public_key();
        let body = format!("sender-key-matrix-{sender_index}").into_bytes();
        let sent = devices[sender_index]
            .engine
            .send_group_payload(
                &group_id,
                body.clone(),
                Some(format!("sender-key-matrix-inner-{sender_index}")),
            )
            .expect("send sender-key matrix group payload");
        assert_eq!(
            sender_key_outer_count(&devices[sender_index].engine, &sent.effects, &sent.event_ids),
            1,
            "sender-key message should publish one shared group outer event"
        );

        let outer_events = sender_key_outer_events_for_engine(
            &devices[sender_index].engine,
            &sent.effects,
            &sent.event_ids,
        )
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
        assert_eq!(outer_events.len(), 1);

        for recipient_index in 0..devices.len() {
            if recipient_index == sender_index {
                continue;
            }
            let (mut received, repair_request_effects) = deliver_protocol_effects_to_engine_once(
                &mut devices[recipient_index].engine,
                &sent.effects,
            );
            if !group_events_contain_body(
                &received,
                &group_id,
                sender_owner,
                sender_device,
                &body,
            ) {
                let (_sender_events, repair_response_effects) =
                    deliver_protocol_effects_to_engine_once(
                        &mut devices[sender_index].engine,
                        &repair_request_effects,
                    );
                let (repaired, _followup) = deliver_protocol_effects_to_engine_once(
                    &mut devices[recipient_index].engine,
                    &repair_response_effects,
                );
                received.extend(repaired);
            }
            assert!(
                group_events_contain_body(
                    &received,
                    &group_id,
                    sender_owner,
                    sender_device,
                    &body,
                ),
                "recipient {recipient_index} did not decrypt message from sender {sender_index}; events={received:?}"
            );

            let duplicate = {
                let mut duplicate_events = Vec::new();
                apply_protocol_events_to_engine(
                    &mut devices[recipient_index].engine,
                    &outer_events,
                    &mut duplicate_events,
                );
                duplicate_events
            };
            assert!(
                !group_events_contain_body(
                    &duplicate,
                    &group_id,
                    sender_owner,
                    sender_device,
                    &body,
                ),
                "duplicate sender-key relay replay emitted a duplicate app message"
            );
        }
    }
}

#[test]
fn appcore_sender_key_late_member_and_remove_member_enforce_membership_window() {
    let mut devices = sender_key_matrix_devices(4);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let dave = 3;
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();
    let dave_owner_pubkey = devices[dave].owner.public_key();
    let alice_owner_pubkey = devices[alice].owner.public_key();
    let alice_device_pubkey = devices[alice].device.public_key();
    let created = devices[alice]
        .engine
        .create_group(
            "sender-key membership window".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(110),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [bob, carol] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }

    let before_add = b"before dave joined".to_vec();
    let before_add_sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            before_add.clone(),
            Some("sender-key-before-add".to_string()),
        )
        .expect("send before late member add");
    let dave_before =
        deliver_protocol_effects_to_engine(&mut devices[dave].engine, &before_add_sent.effects);
    assert!(
        !group_events_contain_body(
            &dave_before,
            &group_id,
            alice_owner_pubkey,
            alice_device_pubkey,
            &before_add
        ),
        "late member must not decrypt messages from before membership"
    );

    let add_dave = devices[alice]
        .engine
        .add_group_members(&group_id, vec![dave_owner_pubkey])
        .expect("add late member");
    for recipient_index in [bob, carol, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &add_dave.effects,
        );
        assert!(
            !group_events_contain_body(
                &events,
                &group_id,
                alice_owner_pubkey,
                alice_device_pubkey,
                &before_add
            ),
            "sender-key distribution on add must not reveal older queued outers"
        );
    }

    let after_add = b"after dave joined".to_vec();
    let after_add_sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            after_add.clone(),
            Some("sender-key-after-add".to_string()),
        )
        .expect("send after late member add");
    for recipient_index in [bob, carol, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &after_add_sent.effects,
        );
        assert!(
            group_events_contain_body(
                &events,
                &group_id,
                alice_owner_pubkey,
                alice_device_pubkey,
                &after_add
            ),
            "current member {recipient_index} did not decrypt post-add sender-key message"
        );
    }

    let remove_bob = devices[alice]
        .engine
        .remove_group_member(&group_id, bob_owner_pubkey)
        .expect("remove member");
    for recipient_index in [bob, carol, dave] {
        deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &remove_bob.effects,
        );
    }

    let after_remove = b"after bob removed".to_vec();
    let after_remove_sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            after_remove.clone(),
            Some("sender-key-after-remove".to_string()),
        )
        .expect("send after member removal");
    let bob_events =
        deliver_protocol_effects_to_engine(&mut devices[bob].engine, &after_remove_sent.effects);
    assert!(
        !group_events_contain_body(
            &bob_events,
            &group_id,
            alice_owner_pubkey,
            alice_device_pubkey,
            &after_remove
        ),
        "removed member must not decrypt future sender-key messages"
    );
    let bob_snapshot = devices[bob].engine.debug_snapshot();
    assert_eq!(
        bob_snapshot.pending_group_sender_key_retry_count, 0,
        "removed member should not request sender-key repair for post-removal outers"
    );
    assert_eq!(
        bob_snapshot.pending_group_sender_key_repair_count, 0,
        "removed member should not keep sender-key repair rows for post-removal outers"
    );
    for recipient_index in [carol, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &after_remove_sent.effects,
        );
        assert!(
            group_events_contain_body(
                &events,
                &group_id,
                alice_owner_pubkey,
                alice_device_pubkey,
                &after_remove
            ),
            "remaining member {recipient_index} did not decrypt post-removal message"
        );
    }
}

#[test]
fn appcore_sender_key_existing_sender_handles_late_add_and_removed_member() {
    let mut devices = sender_key_matrix_devices(4);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let dave = 3;
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();
    let dave_owner_pubkey = devices[dave].owner.public_key();
    let bob_device_pubkey = devices[bob].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key non-actor membership".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(130),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [bob, carol] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }

    let before_add = devices[bob]
        .engine
        .send_group_payload(
            &group_id,
            b"bob before dave".to_vec(),
            Some("sender-key-bob-before-dave".to_string()),
        )
        .expect("bob sends before late member");
    for recipient_index in [alice, carol] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &before_add.effects,
        );
        assert!(
            group_events_contain_body(
                &events,
                &group_id,
                bob_owner_pubkey,
                bob_device_pubkey,
                b"bob before dave"
            ),
            "initial current member {recipient_index} should decrypt bob's sender-key message"
        );
    }

    let add_dave = devices[alice]
        .engine
        .add_group_members(&group_id, vec![dave_owner_pubkey])
        .expect("add late member");
    for recipient_index in [bob, carol, dave] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &add_dave.effects);
    }

    let after_add = devices[bob]
        .engine
        .send_group_payload(
            &group_id,
            b"bob after dave".to_vec(),
            Some("sender-key-bob-after-dave".to_string()),
        )
        .expect("existing member sends after late add");
    let dave_events =
        deliver_protocol_effects_to_engine(&mut devices[dave].engine, &after_add.effects);
    assert!(
        group_events_contain_body(
            &dave_events,
            &group_id,
            bob_owner_pubkey,
            bob_device_pubkey,
            b"bob after dave"
        ),
        "late-added member should decrypt a later send from an existing non-actor member"
    );

    let remove_carol = devices[alice]
        .engine
        .remove_group_member(&group_id, carol_owner_pubkey)
        .expect("remove original member");
    for recipient_index in [bob, carol, dave] {
        deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &remove_carol.effects,
        );
    }

    let after_remove = devices[bob]
        .engine
        .send_group_payload(
            &group_id,
            b"bob after carol removed".to_vec(),
            Some("sender-key-bob-after-carol-removed".to_string()),
        )
        .expect("existing member sends after removal");
    let carol_events =
        deliver_protocol_effects_to_engine(&mut devices[carol].engine, &after_remove.effects);
    assert!(
        !group_events_contain_body(
            &carol_events,
            &group_id,
            bob_owner_pubkey,
            bob_device_pubkey,
            b"bob after carol removed"
        ),
        "removed member must not decrypt future sends from a non-actor existing sender"
    );
    let carol_snapshot = devices[carol].engine.debug_snapshot();
    assert_eq!(
        carol_snapshot.pending_group_sender_key_retry_count, 0,
        "removed member should not request sender-key repair for post-removal outers"
    );
    assert_eq!(
        carol_snapshot.pending_group_sender_key_repair_count, 0,
        "removed member should not keep sender-key repair rows for post-removal outers"
    );
    for recipient_index in [alice, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &after_remove.effects,
        );
        assert!(
            group_events_contain_body(
                &events,
                &group_id,
                bob_owner_pubkey,
                bob_device_pubkey,
                b"bob after carol removed"
            ),
            "remaining member {recipient_index} should decrypt bob's post-removal message"
        );
    }
}

#[test]
fn appcore_sender_key_late_member_repair_denies_pre_join_outer() {
    let mut devices = sender_key_matrix_devices(4);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let dave = 3;
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let bob_device_pubkey = devices[bob].device.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();
    let dave_owner_pubkey = devices[dave].owner.public_key();
    let dave_device_pubkey = devices[dave].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key late repair denied".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(150),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [bob, carol] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }

    let before_add = devices[bob]
        .engine
        .send_group_payload(
            &group_id,
            b"bob pre-dave".to_vec(),
            Some("sender-key-bob-pre-dave".to_string()),
        )
        .expect("bob sends before dave joins");
    let pre_join_outer = sender_key_outer_events_for_engine(
        &devices[bob].engine,
        &before_add.effects,
        &before_add.event_ids,
    )
    .into_iter()
    .next()
    .expect("pre-join sender-key outer")
    .clone();
    for recipient_index in [alice, carol] {
        deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &before_add.effects,
        );
    }

    let add_dave = devices[alice]
        .engine
        .add_group_members(&group_id, vec![dave_owner_pubkey])
        .expect("add late member");
    for recipient_index in [bob, carol, dave] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &add_dave.effects);
    }

    let pending = devices[dave]
        .engine
        .process_group_outer_event(&pre_join_outer)
        .expect("process pre-join outer as late member");
    assert!(pending.consumed);
    assert!(
        devices[dave]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count
            >= 1,
        "pre-join outer should remain pending because dave has no bob sender-key distribution"
    );

    let parsed = parse_group_sender_key_message_event_unchecked(&pre_join_outer).expect("parsed outer");
    let request = nostr_double_ratchet::SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: parsed.sender_event_pubkey,
        key_id: None,
        message_number: None,
        required_revision: None,
        created_at: NdrUnixSeconds(151),
    };
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let repair_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(dave_device_pubkey),
            created_at: NdrUnixSeconds(151),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyRepairRequest { request },
    )
    .expect("repair request payload");
    let response = devices[bob]
        .engine
        .process_group_pairwise_payload(
            &repair_payload,
            dave_owner_pubkey,
            Some(dave_device_pubkey),
        )
        .expect("process late-member pre-join repair request");
    let dave_events =
        deliver_protocol_effects_to_engine(&mut devices[dave].engine, &response.effects);
    assert!(
        !group_events_contain_body(
            &dave_events,
            &group_id,
            bob_owner_pubkey,
            bob_device_pubkey,
            b"bob pre-dave"
        ),
        "late member must not repair-decrypt pre-join sender-key message"
    );
    let dave_snapshot = devices[dave].engine.debug_snapshot();
    assert_eq!(
        dave_snapshot.pending_group_sender_key_retry_count, 0,
        "pre-join sender-key outer should stop requesting repair once the first usable distribution is newer"
    );
    assert_eq!(
        dave_snapshot.pending_group_sender_key_repair_count, 0,
        "pre-join repair request should be cleared with the stale outer"
    );
}

#[test]
fn appcore_sender_key_late_member_repair_allows_post_join_missed_distribution() {
    let mut devices = sender_key_matrix_devices(4);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let dave = 3;
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let bob_device_pubkey = devices[bob].device.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();
    let dave_owner_pubkey = devices[dave].owner.public_key();
    let dave_device_pubkey = devices[dave].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key late repair allowed".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(170),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [bob, carol] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }

    let before_add = devices[bob]
        .engine
        .send_group_payload(
            &group_id,
            b"bob before dave repair split".to_vec(),
            Some("sender-key-bob-before-dave-repair-split".to_string()),
        )
        .expect("bob sends before dave joins");
    for recipient_index in [alice, carol] {
        deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &before_add.effects,
        );
    }

    let add_dave = devices[alice]
        .engine
        .add_group_members(&group_id, vec![dave_owner_pubkey])
        .expect("add late member");
    for recipient_index in [bob, carol, dave] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &add_dave.effects);
    }

    let after_add = devices[bob]
        .engine
        .send_group_payload(
            &group_id,
            b"bob after dave missed distribution".to_vec(),
            Some("sender-key-bob-after-dave-missed-distribution".to_string()),
        )
        .expect("bob sends after dave joins");
    deliver_invite_response_effects_to_engine(&mut devices[dave].engine, &after_add.effects);
    let post_join_outer = sender_key_outer_events_for_engine(
        &devices[bob].engine,
        &after_add.effects,
        &after_add.event_ids,
    )
    .into_iter()
    .next()
    .expect("post-join sender-key outer")
    .clone();

    let pending = devices[dave]
        .engine
        .process_group_outer_event(&post_join_outer)
        .expect("process post-join outer without distribution");
    assert!(pending.consumed);
    assert!(
        devices[dave]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count
            >= 1,
        "post-join outer should remain pending until repair supplies bob's distribution"
    );

    let parsed = parse_group_sender_key_message_event_unchecked(&post_join_outer).expect("parsed outer");
    let request = nostr_double_ratchet::SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: parsed.sender_event_pubkey,
        key_id: None,
        message_number: None,
        required_revision: None,
        created_at: NdrUnixSeconds(171),
    };
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let repair_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(dave_device_pubkey),
            created_at: NdrUnixSeconds(171),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyRepairRequest { request },
    )
    .expect("repair request payload");
    let response = devices[bob]
        .engine
        .process_group_pairwise_payload(
            &repair_payload,
            dave_owner_pubkey,
            Some(dave_device_pubkey),
        )
        .expect("process late-member post-join repair request");
    assert!(
        !response.effects.is_empty(),
        "sender should answer repair when the late member was an intended recipient"
    );

    let dave_events =
        deliver_protocol_effects_to_engine(&mut devices[dave].engine, &response.effects);
    assert!(
        group_events_contain_body(
            &dave_events,
            &group_id,
            bob_owner_pubkey,
            bob_device_pubkey,
            b"bob after dave missed distribution"
        ),
        "late member should repair-decrypt post-join sender-key message"
    );
}

#[test]
fn appcore_sender_key_pending_outer_survives_restart_and_applies_once() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut alice = test_protocol_engine(&alice_owner, &alice_device);
    observe_current_device_appkeys_for_test(&mut alice, &alice_owner, &alice_device);
    let alice_invite = alice.local_invite().expect("alice local invite");

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let mut bob_core = logged_in_test_core_at_data_dir(&bob_owner, &bob_device, data_dir.clone());
    {
        let bob = bob_core
            .protocol_engine
            .as_mut()
            .expect("bob protocol engine");
        observe_current_device_appkeys_for_test(bob, &bob_owner, &bob_device);
        observe_local_invite_for_test(bob, &alice_owner, &alice_device, &alice_invite);
    }
    let bob_invite = bob_core
        .protocol_engine
        .as_ref()
        .expect("bob protocol engine")
        .local_invite()
        .expect("bob local invite");
    observe_local_invite_for_test(&mut alice, &bob_owner, &bob_device, &bob_invite);

    let created = alice
        .create_group(
            "sender-key restart".to_string(),
            vec![bob_owner.public_key()],
            UnixSeconds(122),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    let sent = alice
        .send_group_payload(
            &group_id,
            b"queued across restart".to_vec(),
            Some("sender-key-restart-inner".to_string()),
        )
        .expect("send sender-key message");
    let outer_events = sender_key_outer_events_for_engine(&alice, &sent.effects, &sent.event_ids)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(outer_events.len(), 1);

    {
        let bob = bob_core
            .protocol_engine
            .as_mut()
            .expect("bob protocol engine");
        let pending = bob
            .process_group_outer_event(&outer_events[0])
            .expect("process pending sender-key outer");
        assert!(pending.events.is_empty());
        assert_eq!(
            bob.debug_snapshot().pending_group_sender_key_message_count,
            1
        );
    }

    drop(bob_core);
    let mut restarted = logged_in_test_core_at_data_dir(&bob_owner, &bob_device, data_dir);
    let bob = restarted
        .protocol_engine
        .as_mut()
        .expect("restarted bob protocol engine");
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        1,
        "pending sender-key outer should be durable across restart"
    );
    let applied = deliver_protocol_effects_to_engine(bob, &created.effects);
    assert!(
        group_events_contain_body(
            &applied,
            &group_id,
            alice_owner.public_key(),
            alice_device.public_key(),
            b"queued across restart"
        ),
        "pending sender-key outer should apply after persisted restart and control state arrival"
    );
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        0
    );
    let retry = bob
        .retry_pending_protocol(NdrUnixSeconds(123))
        .expect("retry after persisted pending outer applied");
    assert!(
        retry.group_result.events.is_empty(),
        "applied persisted sender-key outer must not replay"
    );
}

#[test]
#[ignore = "long-running sender-key group membership stress test"]
fn appcore_sender_key_group_membership_stress() {
    let mut devices = sender_key_matrix_devices(6);
    let member_one = devices[1].owner.public_key();
    let member_two = devices[2].owner.public_key();
    let member_three = devices[3].owner.public_key();
    let member_four = devices[4].owner.public_key();
    let created = devices[0]
        .engine
        .create_group(
            "sender-key stress".to_string(),
            vec![member_one, member_two],
            UnixSeconds(200),
        )
        .expect("create sender-key stress group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [1, 2] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }
    let mut active = vec![0usize, 1, 2];

    for step in 0..90 {
        if step == 15 {
            let add = devices[0]
                .engine
                .add_group_members(&group_id, vec![member_three])
                .expect("add fourth stress member");
            active.push(3);
            for recipient_index in active.iter().copied() {
                deliver_protocol_effects_to_engine(
                    &mut devices[recipient_index].engine,
                    &add.effects,
                );
            }
        }
        if step == 35 {
            let add = devices[0]
                .engine
                .add_group_members(&group_id, vec![member_four])
                .expect("add fifth stress member");
            active.push(4);
            for recipient_index in active.iter().copied() {
                deliver_protocol_effects_to_engine(
                    &mut devices[recipient_index].engine,
                    &add.effects,
                );
            }
        }
        if step == 60 {
            let remove = devices[0]
                .engine
                .remove_group_member(&group_id, member_one)
                .expect("remove stress member");
            active.retain(|index| *index != 1);
            for recipient_index in [1usize, 0, 2, 3, 4] {
                deliver_protocol_effects_to_engine(
                    &mut devices[recipient_index].engine,
                    &remove.effects,
                );
            }
        }

        let sender_index = active[step % active.len()];
        let body = format!("sender-key-stress-{step}").into_bytes();
        let sent = devices[sender_index]
            .engine
            .send_group_payload(
                &group_id,
                body.clone(),
                Some(format!("sender-key-stress-inner-{step}")),
            )
            .expect("send stress payload");
        assert_eq!(
            sender_key_outer_count(&devices[sender_index].engine, &sent.effects, &sent.event_ids),
            1
        );
        let sender_owner = devices[sender_index].owner.public_key();
        let sender_device = devices[sender_index].device.public_key();
        for recipient_index in active.iter().copied() {
            if recipient_index == sender_index {
                continue;
            }
            let events = deliver_protocol_effects_to_engine(
                &mut devices[recipient_index].engine,
                &sent.effects,
            );
            assert!(
                group_events_contain_body(&events, &group_id, sender_owner, sender_device, &body),
                "missing stress body at step {step}; sender_index={sender_index}; recipient_index={recipient_index}; active={active:?}; body={}; events={events:?}; recipient_debug={:?}",
                String::from_utf8_lossy(&body),
                devices[recipient_index].engine.debug_snapshot()
            );
        }
        let removed_events =
            deliver_protocol_effects_to_engine(&mut devices[1].engine, &sent.effects);
        if !active.contains(&1) && sender_index != 1 {
            assert!(!group_events_contain_body(
                &removed_events,
                &group_id,
                sender_owner,
                sender_device,
                &body
            ));
        }
    }
}
