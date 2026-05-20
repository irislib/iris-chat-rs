#[test]
fn chat_ttl_applies_to_outgoing_message_expiration() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("outgoing-message-ttl", &owner, &device);
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 60);

    let before = unix_now().get();
    core.send_message(&chat_id, "secret", None);
    let after = unix_now().get();

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("sent message");
    let expires_at = message.expires_at_secs.expect("message expiration");
    assert!(expires_at >= before.saturating_add(60));
    assert!(expires_at <= after.saturating_add(60));
}

#[test]
fn set_chat_message_ttl_action_sets_clears_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir.clone());

    core.handle_action(AppAction::CreateChat {
        peer_input: chat_id.clone(),
    });
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });

    assert_eq!(core.chat_message_ttl_seconds.get(&chat_id), Some(&3600));
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .message_ttl_seconds,
        Some(3600)
    );
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(loaded.chat_message_ttl_seconds.get(&chat_id), Some(&3600));

    let notice_count = core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.len())
        .unwrap_or_default();
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });
    assert_eq!(
        core.threads
            .get(&chat_id)
            .map(|thread| thread.messages.len())
            .unwrap_or_default(),
        notice_count,
        "reselecting the active timer must not publish another chat-settings notice"
    );

    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: None,
    });

    assert!(!core.chat_message_ttl_seconds.contains_key(&chat_id));
    assert_eq!(stored_chat_ttl(&core, &chat_id), None);
    let loaded = core
        .load_persisted()
        .expect("load persisted after clear")
        .expect("persisted state after clear");
    assert!(
        !loaded.chat_message_ttl_seconds.contains_key(&chat_id),
        "cleared ttl is not restored"
    );
}

#[test]
fn send_disappearing_message_action_uses_explicit_expiration_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("send-disappearing-message-action", &owner, &device);
    let expires_at = unix_now().get().saturating_add(600);

    core.handle_action(AppAction::SendDisappearingMessage {
        chat_id: chat_id.clone(),
        text: "secret".to_string(),
        expires_at_secs: expires_at,
    });

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("disappearing message");
    assert_eq!(message.expires_at_secs, Some(expires_at));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &message.id),
        Some(expires_at)
    );
    assert!(
        core.message_expiry_token > 0,
        "expiring sends schedule message pruning"
    );
}

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
    effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event.clone()],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event.clone()],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| publish.event.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn is_non_target_direct_message_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("Invalid header")
        || message.contains("invalid header")
        || message.contains("Failed to decrypt header with available keys")
        || message.contains("invalid HMAC")
}

fn sender_key_outer_count(effects: &[ProtocolEffect], event_ids: &[String]) -> usize {
    protocol_payload_events_for_result(effects, event_ids)
        .into_iter()
        .filter(|event| parse_group_sender_key_message_event(event).is_ok())
        .count()
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

    if parse_group_sender_key_message_event(event).is_ok() {
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
    if parse_group_sender_key_message_event(event).is_ok() {
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
    let outer_events = result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| &publish.event)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .filter(|event| parse_group_sender_key_message_event(event).is_ok())
        .collect::<Vec<_>>();

    assert_eq!(
        outer_events.len(),
        1,
        "sender-key group send should publish one shared outer event"
    );
    assert_eq!(outer_events[0].id.to_string(), result.event_ids[0]);
    assert_eq!(
        parse_group_sender_key_message_event(outer_events[0])
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
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
        .expect("sender-key outer event");

    let pending = bob
        .process_group_outer_event(outer)
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
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
        .expect("sender-key outer event");

    let pending = devices[bob]
        .engine
        .process_group_outer_event(outer)
        .expect("process outer missing rotated key");
    assert!(pending.pending);
    assert_eq!(
        devices[bob]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        1
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
            .pending_group_sender_key_message_count,
        0
    );
}

#[test]
fn appcore_sender_key_repair_request_survives_restart_and_throttles() {
    let bob_storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut devices = (0..3)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    devices[1].engine = test_protocol_engine_with_storage(
        &devices[1].owner,
        &devices[1].device,
        bob_storage.clone(),
    );
    observe_sender_key_matrix_protocol_state(&mut devices);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let bob_owner = devices[bob].owner.clone();
    let bob_device = devices[bob].device.clone();
    let bob_owner_pubkey = bob_owner.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key repair restart".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(304),
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
            b"repair after restart".to_vec(),
            Some("sender-key-repair-restart-inner".to_string()),
        )
        .expect("send with rotated sender key");
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
        .expect("sender-key outer event");
    let pending = devices[bob]
        .engine
        .process_group_outer_event(outer)
        .expect("process outer missing rotated key");
    assert!(!pending.effects.is_empty());
    let before_restart = devices[bob].engine.debug_snapshot();
    assert_eq!(before_restart.pending_group_sender_key_repair_count, 1);
    assert!(before_restart.pending_group_sender_key_repair_last_requested_at_secs > 0);

    devices[bob].engine =
        test_protocol_engine_with_storage(&bob_owner, &bob_device, bob_storage.clone());
    let after_restart = devices[bob].engine.debug_snapshot();
    assert_eq!(after_restart.pending_group_sender_key_repair_count, 1);
    assert_eq!(
        after_restart.pending_group_sender_key_repair_last_requested_at_secs,
        before_restart.pending_group_sender_key_repair_last_requested_at_secs
    );

    let early = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_restart
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(1),
        ))
        .expect("early retry");
    assert!(
        early.group_result.effects.is_empty(),
        "repair request should be throttled before retry delay"
    );

    let late = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_restart
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(31),
        ))
        .expect("late retry");
    assert!(
        !late.group_result.effects.is_empty(),
        "repair request should be re-emitted after retry delay"
    );

    let after_late_retry = devices[bob].engine.debug_snapshot();
    let second_early = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_late_retry
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(31),
        ))
        .expect("second early retry");
    assert!(
        second_early.group_result.effects.is_empty(),
        "repair request should back off after the second request"
    );

    let second_late = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_late_retry
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(121),
        ))
        .expect("second late retry");
    assert!(
        !second_late.group_result.effects.is_empty(),
        "repair request should re-emit after the backoff delay"
    );
}

#[test]
fn appcore_sender_key_missing_metadata_revision_repairs_and_applies_pending_outer() {
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
            "sender-key metadata repair".to_string(),
            vec![bob_owner, carol_owner],
            UnixSeconds(320),
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

    let distribution =
        latest_sender_key_distribution_for_test(&devices[alice].engine, &group_id, NdrUnixSeconds(321));
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let distribution_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device),
            created_at: NdrUnixSeconds(321),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyDistribution { distribution },
    )
    .expect("sender-key distribution payload");
    devices[bob]
        .engine
        .process_group_pairwise_payload(&distribution_payload, alice_owner, Some(alice_device))
        .expect("process rotated distribution without metadata");

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"after missed metadata".to_vec(),
            Some("sender-key-metadata-repair-inner".to_string()),
        )
        .expect("send after metadata gap");
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
        .expect("sender-key outer event");

    let pending = devices[bob]
        .engine
        .process_group_outer_event(outer)
        .expect("process outer missing metadata revision");
    assert!(pending.pending);
    assert!(
        !pending.effects.is_empty(),
        "missing metadata revision should request repair"
    );

    let (_alice_events, metadata_response_effects) =
        deliver_protocol_effects_to_engine_once(&mut devices[alice].engine, &pending.effects);
    assert!(
        !metadata_response_effects.is_empty(),
        "sender should answer revision repair with metadata"
    );
    let (bob_events, _followup) = deliver_protocol_effects_to_engine_once(
        &mut devices[bob].engine,
        &metadata_response_effects,
    );
    assert!(
        group_events_contain_body(
            &bob_events,
            &group_id,
            alice_owner,
            alice_device,
            b"after missed metadata"
        ),
        "pending sender-key outer should apply after metadata repair"
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
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
        .expect("sender-key outer event");
    let pending = devices[bob]
        .engine
        .process_group_outer_event(outer)
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
    assert!(
        !group_events_contain_body(
            &bob_after_key_events,
            &group_id,
            alice_owner_pubkey,
            alice_device_pubkey,
            b"repair after sender restart"
        ),
        "key repair should still wait for metadata repair"
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
        key_id: distribution.key_id,
        message_number: 0,
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
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
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
        2,
        "remaining member should receive metadata and rotated sender key"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &carol_owner.public_key().to_hex()),
        1,
        "removed member should receive metadata but not the rotated sender key"
    );
}

#[test]
fn appcore_existing_pairwise_group_still_uses_pairwise_fanout() {
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
    snapshot.protocol = nostr_double_ratchet::GroupProtocol::PairwiseFanoutV1;
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
    engine
        .process_group_pairwise_payload(
            &metadata_payload,
            owner.public_key(),
            Some(device.public_key()),
        )
        .expect("install legacy pairwise group");

    let result = engine
        .send_group_payload(
            &group_id,
            b"legacy pairwise body".to_vec(),
            Some("legacy-inner".to_string()),
        )
        .expect("send legacy pairwise group payload");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(result.event_ids.len(), 1);
    assert_eq!(payload_events.len(), 1);
    assert!(parse_message_event(payload_events[0]).is_ok());
    assert!(parse_group_sender_key_message_event(payload_events[0]).is_err());
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &peer_owner.public_key().to_hex()),
        1
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
            sender_key_outer_count(&sent.effects, &sent.event_ids),
            1,
            "sender-key message should publish one shared group outer event"
        );

        let outer_events = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
            .into_iter()
            .filter(|event| parse_group_sender_key_message_event(event).is_ok())
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(outer_events.len(), 1);

        for recipient_index in 0..devices.len() {
            if recipient_index == sender_index {
                continue;
            }
            let received = deliver_protocol_effects_to_engine(
                &mut devices[recipient_index].engine,
                &sent.effects,
            );
            assert!(
                group_events_contain_body(
                    &received,
                    &group_id,
                    sender_owner,
                    sender_device,
                    &body
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
                    &body
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
    let pre_join_outer =
        protocol_payload_events_for_result(&before_add.effects, &before_add.event_ids)
            .into_iter()
            .find(|event| parse_group_sender_key_message_event(event).is_ok())
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
    assert_eq!(
        devices[dave]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        1,
        "pre-join outer should remain pending because dave has no bob sender-key distribution"
    );

    let parsed = parse_group_sender_key_message_event(&pre_join_outer).expect("parsed outer");
    let request = nostr_double_ratchet::SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: parsed.sender_event_pubkey,
        key_id: parsed.key_id,
        message_number: parsed.message_number,
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
    assert!(
        response.effects.is_empty(),
        "sender must not answer late-member repair for a pre-join sender-key message"
    );
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
    let post_join_outer =
        protocol_payload_events_for_result(&after_add.effects, &after_add.event_ids)
            .into_iter()
            .find(|event| parse_group_sender_key_message_event(event).is_ok())
            .expect("post-join sender-key outer")
            .clone();

    let pending = devices[dave]
        .engine
        .process_group_outer_event(&post_join_outer)
        .expect("process post-join outer without distribution");
    assert!(pending.consumed);
    assert_eq!(
        devices[dave]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        1,
        "post-join outer should remain pending until repair supplies bob's distribution"
    );

    let parsed = parse_group_sender_key_message_event(&post_join_outer).expect("parsed outer");
    let request = nostr_double_ratchet::SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: parsed.sender_event_pubkey,
        key_id: parsed.key_id,
        message_number: parsed.message_number,
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
    let outer_events = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .filter(|event| parse_group_sender_key_message_event(event).is_ok())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(outer_events.len(), 1);

    {
        let bob = bob_core
            .protocol_engine
            .as_mut()
            .expect("bob protocol engine");
        let mut pending_events = Vec::new();
        apply_protocol_events_to_engine(bob, &outer_events, &mut pending_events);
        assert!(pending_events.is_empty());
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
        assert_eq!(sender_key_outer_count(&sent.effects, &sent.event_ids), 1);
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

/// Repro for the macOS / Android bug: tapping a direct chat title pushes
/// the info screen, and back must return to the chat instead of the chat
/// list. Mirrors the group-details flow but for the direct-message case
/// after we converted both UIs from local overlays to the router push.
#[test]
fn back_from_direct_chat_info_returns_to_chat() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("back-from-direct-info", &owner, &device);
    let chat_id = peer.public_key().to_hex();

    core.handle_action(AppAction::OpenChat {
        chat_id: chat_id.clone(),
    });
    assert_eq!(
        core.state.router.screen_stack,
        vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }],
        "chat opened"
    );

    core.handle_action(AppAction::PushScreen {
        screen: Screen::DirectChatInfo {
            chat_id: chat_id.clone(),
        },
    });
    assert_eq!(
        core.state.router.screen_stack,
        vec![
            Screen::Chat {
                chat_id: chat_id.clone(),
            },
            Screen::DirectChatInfo {
                chat_id: chat_id.clone(),
            },
        ],
        "info pushed on top of the chat"
    );

    let mut next_stack = core.state.router.screen_stack.clone();
    next_stack.pop();
    core.handle_action(AppAction::UpdateScreenStack { stack: next_stack });

    assert_eq!(
        core.state.router.screen_stack,
        vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }],
        "back tap returns to the chat"
    );
    assert_eq!(
        core.active_chat_id.as_deref(),
        Some(chat_id.as_str()),
        "active chat is restored from the router"
    );
}

/// Repro for the Android bug: opening group details from a chat and then
/// pressing back must return to the chat — not jump to the chat list.
/// The Android UI sends `UpdateScreenStack(stack.dropLast())` for back
/// taps, so a `[Chat, GroupDetails]` → `[Chat]` round-trip through the
/// core has to keep `Chat` on the stack.
#[test]
fn back_from_group_details_returns_to_chat() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("back-from-group-details", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Crew".to_string(),
        member_inputs: Vec::new(),
    });

    let group_id = core
        .state
        .current_chat
        .as_ref()
        .and_then(|chat| chat.group_id.clone())
        .expect("group id");
    let group_chat_id = format!("group:{group_id}");

    // After CreateGroup the chat is already active. Push group details
    // as the chat title tap would.
    core.handle_action(AppAction::PushScreen {
        screen: Screen::GroupDetails {
            group_id: group_id.clone(),
        },
    });
    assert_eq!(
        core.state.router.screen_stack,
        vec![
            Screen::Chat {
                chat_id: group_chat_id.clone(),
            },
            Screen::GroupDetails {
                group_id: group_id.clone(),
            },
        ],
        "details pushed on top of the chat"
    );

    // Mimic Android's back tap: drop the last screen and let the core
    // reconcile via UpdateScreenStack.
    let mut next_stack = core.state.router.screen_stack.clone();
    next_stack.pop();
    core.handle_action(AppAction::UpdateScreenStack { stack: next_stack });

    assert_eq!(
        core.state.router.screen_stack,
        vec![Screen::Chat {
            chat_id: group_chat_id.clone(),
        }],
        "back tap returns to the chat, not the chat list"
    );
    assert_eq!(
        core.active_chat_id.as_deref(),
        Some(group_chat_id.as_str()),
        "active chat is restored from the router"
    );
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .map(|chat| chat.chat_id.clone()),
        Some(group_chat_id),
        "projection re-emits current_chat on back"
    );
}

#[test]
fn create_group_allows_self_only_group() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("self-only-group", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Notes".to_string(),
        member_inputs: Vec::new(),
    });

    let current = core.state.current_chat.as_ref().expect("opened group chat");
    let group_id = current.group_id.as_ref().expect("group id").clone();
    let group = core.groups.get(&group_id).expect("stored group");
    let owner = ndr_owner_pubkey(owner.public_key());
    assert_eq!(group.name, "Notes");
    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(group.members, vec![owner]);
    assert_eq!(group.admins, vec![owner]);
}

#[test]
fn group_picture_projects_to_chat_list_current_chat_and_details() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-picture-projection", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Photo Group".to_string(),
        member_inputs: Vec::new(),
    });

    let current = core.state.current_chat.as_ref().expect("opened group chat");
    let group_id = current.group_id.as_ref().expect("group id").clone();
    let chat_id = group_chat_id(&group_id);
    let picture_url = "htree://nhash1group/photo.jpg".to_string();
    core.set_group_picture(&group_id, Some(picture_url.clone()));

    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .and_then(|chat| chat.picture_url.as_deref()),
        Some(picture_url.as_str())
    );
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .and_then(|chat| chat.picture_url.as_deref()),
        Some(picture_url.as_str())
    );

    core.screen_stack = vec![Screen::GroupDetails {
        group_id: group_id.clone(),
    }];
    core.rebuild_state();
    assert_eq!(
        core.state
            .group_details
            .as_ref()
            .and_then(|details| details.picture_url.as_deref()),
        Some(picture_url.as_str())
    );
}

/// Picture lives inside the protocol's `GroupSnapshot` now (ndr >=0.0.144),
/// so setting one and reloading must round-trip the field through the
/// engine's persisted group_json — not via the legacy `group_pictures` map.
#[test]
fn group_picture_persists_inside_protocol_group_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir);

    core.handle_action(AppAction::CreateGroup {
        name: "Persisted Photo Group".to_string(),
        member_inputs: Vec::new(),
    });

    let group_id = core
        .state
        .current_chat
        .as_ref()
        .and_then(|chat| chat.group_id.clone())
        .expect("group id");
    let picture_url = "htree://nhash1persisted/photo%201.jpg".to_string();
    core.set_group_picture(&group_id, Some(picture_url.clone()));

    let persisted = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(
        persisted
            .groups
            .iter()
            .find(|group| group.group_id == group_id)
            .and_then(|group| group.picture.as_deref()),
        Some(picture_url.as_str()),
        "picture lives on the persisted GroupSnapshot, not in a side table"
    );
}

/// Member changes from a peer admin arrive as a fresh `MetadataUpdated`
/// snapshot. With ndr >=0.0.144 the picture is part of that snapshot, so
/// preservation across membership updates is the peer admin's responsibility:
/// they must include the current picture in the snapshot they broadcast.
/// This test pins down the local apply behavior — what we render must
/// reflect whatever the latest snapshot says.
#[test]
fn group_picture_follows_metadata_snapshot_on_incoming_changes() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-picture-membership", &owner, &device);
    let group_id = "group-picture-membership".to_string();
    let owner_pubkey = owner.public_key();
    let new_member = Keys::generate().public_key();
    let picture_url = "htree://nhash1retained/photo.jpg".to_string();

    // Seed: single-member group with a picture, as a peer admin would have
    // broadcast it (revision 1 carries the picture).
    let mut initial = test_group_snapshot(
        &group_id,
        "Photos",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        1,
    );
    initial.picture = Some(picture_url.clone());
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(initial.clone()));

    // Peer admin adds a member: a well-behaved admin keeps the picture set
    // in the new revision's snapshot, so members on the other end keep
    // seeing it.
    let mut after_add = test_group_snapshot(
        &group_id,
        "Photos",
        owner_pubkey,
        vec![owner_pubkey, new_member],
        vec![owner_pubkey],
        2,
    );
    after_add.picture = Some(picture_url.clone());
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(after_add));

    core.rebuild_state();
    let chat_id = group_chat_id(&group_id);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .and_then(|chat| chat.picture_url.as_deref()),
        Some(picture_url.as_str()),
        "picture set on the new revision must show up in chat list"
    );
}

#[test]
fn group_metadata_changes_create_system_notices() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-metadata-notices", &owner, &device);
    let group_id = "group-notice-test".to_string();
    let chat_id = group_chat_id(&group_id);
    let owner_pubkey = owner.public_key();
    let member = Keys::generate().public_key();
    let initial = test_group_snapshot(
        &group_id,
        "Original",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        1,
    );
    let renamed = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        2,
    );
    let with_member = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey, member],
        vec![owner_pubkey],
        3,
    );
    let member_removed = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        4,
    );

    core.apply_group_metadata_notice(None, &initial);
    core.apply_group_metadata_notice(Some(&initial), &renamed);
    core.apply_group_metadata_notice(Some(&renamed), &with_member);
    core.apply_group_metadata_notice(Some(&with_member), &member_removed);
    let with_admin = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey, member],
        vec![owner_pubkey, member],
        5,
    );
    core.apply_group_metadata_notice(Some(&with_member), &with_admin);

    let messages = &core.threads.get(&chat_id).expect("group thread").messages;
    assert!(messages
        .iter()
        .any(|message| message.body == "Group created: Original"));
    assert!(messages
        .iter()
        .any(|message| message.body == "Group renamed to Renamed"));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("joined the group")));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("left the group")));
    assert!(messages
        .iter()
        .any(|message| message.kind == ChatMessageKind::System));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("became an admin")));
}

#[test]
fn appcore_restart_restores_threads_groups_and_seen_events() {
    // End-to-end check that the persistence/load round trip survives a
    // full AppCore drop+recreate against the same `data_dir`.
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir_str = temp_dir.path().to_string_lossy().to_string();

    let chat_id = "deadbeef".repeat(8);
    let group_id = "group-restart".to_string();
    let group_chat = group_chat_id(&group_id);

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            data_dir_str.clone(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: Vec::new(),
            authorization_state: LocalAuthorizationState::Authorized,
        });

        core.next_message_id = 17;
        core.active_chat_id = Some(chat_id.clone());
        core.app_keys.insert(
            owner.public_key().to_hex(),
            known_app_keys_from_ndr(
                owner.public_key(),
                &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 5)]),
                10,
            ),
        );
        core.groups.insert(
            group_id.clone(),
            test_group_snapshot(
                &group_id,
                "Brunch",
                owner.public_key(),
                vec![owner.public_key()],
                vec![owner.public_key()],
                1_000,
            ),
        );
        core.threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 3,
                updated_at_secs: 200,
                messages: vec![
                    ChatMessageSnapshot {
                        id: "m1".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: owner.public_key().to_hex(),
                        author_owner_pubkey_hex: Some(owner.public_key().to_hex()),
                        author_picture_url: None,
                        body: "hello world".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: true,
                        created_at_secs: 100,
                        expires_at_secs: None,
                        delivery: DeliveryState::Sent,
                        recipient_deliveries: Vec::new(),
                        delivery_trace: Default::default(),
                        source_event_id: None,
                    },
                    ChatMessageSnapshot {
                        id: "m2".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: "peer".to_string(),
                        author_owner_pubkey_hex: None,
                        author_picture_url: None,
                        body: "right back atcha".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: false,
                        created_at_secs: 110,
                        expires_at_secs: None,
                        delivery: DeliveryState::Received,
                        recipient_deliveries: Vec::new(),
                        delivery_trace: Default::default(),
                        source_event_id: None,
                    },
                ],

                draft: String::new(),
            },
        );
        core.threads.insert(
            group_chat.clone(),
            ThreadRecord {
                chat_id: group_chat.clone(),
                unread_count: 0,
                updated_at_secs: 50,
                messages: vec![ChatMessageSnapshot {
                    id: "g-system".to_string(),
                    chat_id: group_chat.clone(),
                    kind: ChatMessageKind::System,
                    author: owner.public_key().to_hex(),
                    author_owner_pubkey_hex: Some(owner.public_key().to_hex()),
                    author_picture_url: None,
                    body: "Group created: Brunch".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 50,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                }],

                draft: String::new(),
            },
        );
        core.seen_event_order.push_back("evt-1".to_string());
        core.seen_event_order.push_back("evt-2".to_string());
        core.seen_event_ids = core.seen_event_order.iter().cloned().collect();
        core.preferences.send_typing_indicators = true;
        core.preferences.nearby_bluetooth_enabled = true;
        core.preferences.nearby_lan_enabled = true;

        core.persist_best_effort_inner();
    }

    // New AppCore over the same directory: load_persisted should return
    // exactly what we wrote.
    let mut restarted = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir_str,
        Arc::new(RwLock::new(AppState::empty())),
    );
    let loaded = restarted
        .load_persisted()
        .expect("load_persisted")
        .expect("state persisted");
    assert_eq!(loaded.next_message_id, 17);
    assert_eq!(loaded.active_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(loaded.preferences.send_typing_indicators);
    assert!(loaded.preferences.nearby_bluetooth_enabled);
    assert!(loaded.preferences.nearby_lan_enabled);
    assert_eq!(loaded.threads.len(), 2);
    let dm_thread = loaded
        .threads
        .iter()
        .find(|thread| thread.chat_id == chat_id)
        .expect("dm thread present");
    assert_eq!(dm_thread.messages.len(), 2);
    assert_eq!(dm_thread.unread_count, 3);
    assert_eq!(dm_thread.messages[0].body, "hello world");
    assert_eq!(dm_thread.messages[1].body, "right back atcha");
    let group_thread = loaded
        .threads
        .iter()
        .find(|thread| thread.chat_id == group_chat)
        .expect("group thread present");
    assert!(matches!(
        group_thread.messages[0].kind,
        ChatMessageKind::System
    ));
    assert_eq!(loaded.groups.len(), 1);
    assert_eq!(loaded.groups[0].name, "Brunch");
    assert_eq!(loaded.app_keys.len(), 1);
    assert_eq!(loaded.seen_event_ids, vec!["evt-1", "evt-2"]);
    assert!(matches!(
        loaded.authorization_state,
        Some(PersistedAuthorizationState::Authorized)
    ));

    // Assert nothing was persisted in the legacy JSON layout.
    let legacy_meta = std::path::Path::new(&restarted.data_dir)
        .join("core")
        .join("meta.json");
    assert!(
        !legacy_meta.exists(),
        "legacy core/meta.json must not be created"
    );
}

#[test]
fn appcore_clear_persistence_drops_sqlite_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir_str = temp_dir.path().to_string_lossy().to_string();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir_str.clone(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.next_message_id = 5;
    core.persist_best_effort_inner();
    assert!(core.load_persisted().unwrap().is_some());

    core.clear_persistence_best_effort();
    assert!(core.load_persisted().unwrap().is_none());
}

#[test]
fn profile_picture_upload_propagates_to_account_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("profile-picture-upload", &owner, &device);
    core.rebuild_state();
    assert!(core.state.account.is_some(), "account snapshot exists");
    assert!(
        core.state.account.as_ref().unwrap().picture_url.is_none(),
        "no picture before upload"
    );

    let picture_url = "https://cdn.iris.to/abc123".to_string();
    core.handle_profile_picture_upload_finished(Ok(picture_url.clone()));

    let account = core.state.account.as_ref().expect("account after upload");
    assert_eq!(
        account.picture_url.as_deref(),
        Some(picture_url.as_str()),
        "picture url propagated to account snapshot"
    );
}

#[test]
fn delete_chat_removes_thread_and_navigates_back() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("delete-chat", &owner, &device);
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 100,
            messages: vec![ChatMessageSnapshot {
                id: "m1".to_string(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: chat_id.clone(),
                author_owner_pubkey_hex: Some(chat_id.clone()),
                author_picture_url: None,
                body: "hi".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 100,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: None,
            }],

            draft: String::new(),
        },
    );
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 3600);
    core.preferences.pinned_chat_ids.push(chat_id.clone());
    core.active_chat_id = Some(chat_id.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];

    core.handle_action(AppAction::DeleteChat {
        chat_id: chat_id.clone(),
    });

    assert!(!core.threads.contains_key(&chat_id), "thread removed");
    assert!(
        !core.chat_message_ttl_seconds.contains_key(&chat_id),
        "ttl cleared"
    );
    assert!(
        !core
            .preferences
            .pinned_chat_ids
            .iter()
            .any(|pinned| pinned == &chat_id),
        "pinned state cleared"
    );
    assert!(core.active_chat_id.is_none(), "active chat cleared");
    assert!(
        !core
            .screen_stack
            .iter()
            .any(|s| matches!(s, Screen::Chat { chat_id: cid } if cid == &chat_id)),
        "chat screen popped"
    );
    assert!(
        !core
            .state
            .chat_list
            .iter()
            .any(|chat| chat.chat_id == chat_id),
        "chat_list snapshot reflects removal"
    );
}

#[test]
fn pinning_chat_moves_it_above_newer_unpinned_chats_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("pin-chat", &owner, &device);
    let older_chat_id = Keys::generate().public_key().to_hex();
    let newer_chat_id = Keys::generate().public_key().to_hex();
    core.threads.insert(
        older_chat_id.clone(),
        ThreadRecord {
            chat_id: older_chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: vec![test_chat_message(
                &older_chat_id,
                "older",
                "older",
                10,
                false,
            )],

            draft: String::new(),
        },
    );
    core.threads.insert(
        newer_chat_id.clone(),
        ThreadRecord {
            chat_id: newer_chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 20,
            messages: vec![test_chat_message(
                &newer_chat_id,
                "newer",
                "newer",
                20,
                false,
            )],

            draft: String::new(),
        },
    );
    core.rebuild_state();
    assert_eq!(core.state.chat_list[0].chat_id, newer_chat_id);

    core.handle_action(AppAction::SetChatPinned {
        chat_id: older_chat_id.clone(),
        pinned: true,
    });

    assert_eq!(core.state.chat_list[0].chat_id, older_chat_id);
    assert!(core.state.chat_list[0].is_pinned);
    assert_eq!(core.state.chat_list[1].chat_id, newer_chat_id);
    assert!(!core.state.chat_list[1].is_pinned);
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("state persisted");
    assert_eq!(loaded.preferences.pinned_chat_ids, vec![older_chat_id]);
}

#[test]
fn set_chat_unread_toggles_local_unread_count() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("set-chat-unread", &owner, &device);
    let chat_id = Keys::generate().public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: vec![test_chat_message(&chat_id, "m1", "hello", 10, false)],

            draft: String::new(),
        },
    );

    core.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: true,
    });

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 1);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat snapshot")
            .unread_count,
        1
    );

    core.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: false,
    });

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 0);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat snapshot")
            .unread_count,
        0
    );
}

#[test]
fn redelivered_persisted_message_after_restart_does_not_increment_unread() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("redelivered-persisted-message", &owner, &device);
    let old_message = ChatMessageSnapshot {
        id: "old-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        author_owner_pubkey_hex: Some(chat_id.clone()),
        author_picture_url: None,
        body: "already read".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 100,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("outer-old".to_string()),
    };
    let latest_message = ChatMessageSnapshot {
        id: "latest-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        author_owner_pubkey_hex: Some(chat_id.clone()),
        author_picture_url: None,
        body: "latest preview".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 200,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("outer-latest".to_string()),
    };
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 200,
            messages: vec![old_message, latest_message.clone()],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    // Restart restores only a preview for inactive chats. Catch-up can then
    // redeliver older stored events that are not in memory anymore.
    let thread = core.threads.get_mut(&chat_id).expect("thread");
    thread.messages = vec![latest_message];
    thread.unread_count = 0;
    core.active_chat_id = None;
    core.screen_stack.clear();

    core.push_incoming_message_from(
        &chat_id,
        Some("old-message".to_string()),
        "already read".to_string(),
        100,
        None,
        Some(chat_id.clone()),
        Some(chat_id.clone()),
        Some("outer-old".to_string()),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 0);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(stored_message_count(&core), 2);
}

#[test]
fn prune_expired_messages_removes_loaded_messages_and_sqlite_rows() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("message-expiry-prune", &owner, &device);
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 200,
            messages: vec![
                ChatMessageSnapshot {
                    id: "expired".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 100,
                    expires_at_secs: Some(150),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 200,
                    expires_at_secs: Some(300),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
            ],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    let removed = core.prune_expired_messages(200);

    assert_eq!(removed, 1);
    assert_eq!(stored_message_count(&core), 1);
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 1);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "stays");
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .messages
            .len(),
        1
    );
}

/// Regression for iOS RUNNINGBOARD 0xdead10cc crashes: a relay event
/// queued just before `PrepareForSuspend` (or one that races in from
/// the FFI channel during the suspend window) used to keep running
/// inside `handle_relay_event_with_channel` and write to SQLite,
/// which iOS' watchdog kills mid-`pwrite`. After this fix the gate
/// in `handle_internal` drops queued background work once
/// `PrepareForSuspend` has run, and clears on `AppForegrounded`.
#[test]
fn suspend_gate_drops_internal_events_until_foregrounded() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    // DebugLog is a simple internal-event signal: the handler appends
    // to `core.debug_log`, no other code path mutates it, and it's
    // not pruned by `rebuild_state` like typing indicators are.
    let send_debug_log = |core: &mut AppCore, detail: &str| {
        core.handle_message(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
            category: "test".to_string(),
            detail: detail.to_string(),
        })));
    };
    let log_count = |core: &AppCore| -> usize {
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "test")
            .count()
    };

    // Sanity: an internal event before suspend lands in debug_log.
    send_debug_log(&mut core, "before-suspend");
    assert_eq!(log_count(&core), 1, "DebugLog must land before suspend");

    // Engage the gate via the real CoreMsg path that iOS uses.
    let (reply_tx, _reply_rx) = flume::bounded(1);
    core.handle_message(CoreMsg::PrepareForSuspend(reply_tx));

    // While suspended, internal events must be dropped — the gate is
    // what keeps SQLite from being written while iOS is killing us.
    send_debug_log(&mut core, "during-suspend");
    assert_eq!(
        log_count(&core),
        1,
        "suspend gate must drop internal events"
    );

    // Foregrounding lifts the gate; the next internal event is processed.
    core.handle_message(CoreMsg::Action(AppAction::AppForegrounded));
    send_debug_log(&mut core, "after-foreground");
    assert_eq!(
        log_count(&core),
        2,
        "after AppForegrounded, internal events flow again"
    );
}

#[test]
fn internal_prune_expired_messages_event_ignores_stale_tokens_and_updates_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("message-expiry-internal-event", &owner, &device);
    let now = unix_now().get();
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 1,
            updated_at_secs: now,
            messages: vec![
                ChatMessageSnapshot {
                    id: "expired".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: now.saturating_sub(20),
                    expires_at_secs: Some(now.saturating_sub(1)),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: now,
                    expires_at_secs: Some(now.saturating_add(3600)),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
            ],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    core.handle_prune_expired_messages(core.message_expiry_token.wrapping_add(1));

    assert_eq!(stored_message_count(&core), 2, "stale token ignored");
    assert_eq!(
        core.threads
            .get(&chat_id)
            .expect("thread after stale token")
            .messages
            .len(),
        2
    );

    let valid_token = core.message_expiry_token;
    core.handle_prune_expired_messages(valid_token);

    assert_eq!(stored_message_count(&core), 1);
    let thread = core.threads.get(&chat_id).expect("thread after prune");
    assert_eq!(thread.unread_count, 0);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "stays");
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat after prune")
            .messages
            .len(),
        1
    );
}

fn logged_in_test_core(label: &str, owner: &Keys, device: &Keys) -> AppCore {
    logged_in_test_core_at_data_dir(
        owner,
        device,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-{label}-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
    )
}

fn logged_in_test_core_with_storage(
    label: &str,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-{label}-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    core
}

fn logged_in_test_core_at_data_dir(owner: &Keys, device: &Keys, data_dir: String) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    core
}

fn test_chat_message(
    chat_id: &str,
    id: &str,
    body: &str,
    created_at_secs: u64,
    is_outgoing: bool,
) -> ChatMessageSnapshot {
    ChatMessageSnapshot {
        id: id.to_string(),
        chat_id: chat_id.to_string(),
        kind: ChatMessageKind::User,
        author: chat_id.to_string(),
        author_owner_pubkey_hex: Some(chat_id.to_string()),
        author_picture_url: None,
        body: body.to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing,
        created_at_secs,
        expires_at_secs: None,
        delivery: if is_outgoing {
            DeliveryState::Sent
        } else {
            DeliveryState::Received
        },
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: None,
    }
}

fn test_protocol_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    test_protocol_engine_with_storage(owner, device, storage)
}

fn test_protocol_engine_with_storage(
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> ProtocolEngine {
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_invite = stable_local_invite_for_test(owner, device);
    let mut session_manager =
        SessionManager::new(local_owner, device.secret_key().to_secret_bytes()).snapshot();
    session_manager.local_invite = Some(local_invite);
    let group_manager = NostrGroupManager::new(local_owner).snapshot();
    ProtocolEngine::seed_storage_if_missing_for_test(storage.as_ref(), session_manager, group_manager)
        .expect("seed protocol state");
    ProtocolEngine::load_or_create_for_local_device(
        storage,
        owner.public_key(),
        device,
    )
    .expect("protocol engine")
}

fn observe_current_device_appkeys_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
) {
    let created_at = unix_now().get();
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), created_at)]),
            created_at,
        )
        .expect("local appkeys");
}

fn observe_peer_device_invite_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
    created_at: u64,
) {
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), created_at)]),
            created_at,
        )
        .expect("peer appkeys");
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(created_at), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        ndr_device_pubkey(device.public_key()),
        Some(ndr_owner_pubkey(owner.public_key())),
        None,
    )
    .expect("peer invite");
    let event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(device)
        .expect("signed invite");
    engine
        .observe_invite_event(&event)
        .expect("observe peer invite");
}

fn protocol_payload_events_for_result<'a>(
    effects: &'a [ProtocolEffect],
    event_ids: &[String],
) -> Vec<&'a Event> {
    let event_ids = event_ids.iter().cloned().collect::<HashSet<_>>();
    protocol_effect_events(effects)
        .into_iter()
        .filter(|event| event_ids.contains(&event.id.to_string()))
        .collect()
}

fn protocol_effect_events(effects: &[ProtocolEffect]) -> Vec<&Event> {
    effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| &publish.event)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn protocol_targeted_payload_count(effects: &[ProtocolEffect], owner_pubkey_hex: &str) -> usize {
    effects
        .iter()
        .map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                target_owner_pubkey_hex,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(owner_pubkey_hex) => 1,
            ProtocolEffect::PublishStagedFirstContact { payload, .. } => payload
                .iter()
                .filter(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(owner_pubkey_hex)
                })
                .count(),
            _ => 0,
        })
        .sum()
}

fn latest_sender_key_distribution_for_test(
    engine: &ProtocolEngine,
    group_id: &str,
    created_at: NdrUnixSeconds,
) -> nostr_double_ratchet::SenderKeyDistribution {
    let sender_key = engine
        .group_manager_snapshot_for_test()
        .sender_keys
        .into_iter()
        .find(|record| record.group_id == group_id)
        .expect("sender-key record for group");
    let key_id = sender_key.latest_key_id.expect("latest sender key id");
    let state = sender_key
        .states
        .iter()
        .find(|state| state.key_id() == key_id)
        .expect("sender-key state");
    nostr_double_ratchet::SenderKeyDistribution {
        group_id: group_id.to_string(),
        key_id,
        sender_event_pubkey: sender_key.sender_event_pubkey,
        chain_key: state.chain_key(),
        iteration: state.iteration(),
        created_at,
    }
}

fn install_test_protocol_engine(
    core: &mut AppCore,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
    seed_session_manager: Option<SessionManagerSnapshot>,
    seed_group_manager: Option<GroupManagerSnapshot>,
) {
    let local_invite = stable_local_invite_for_test(owner, device);
    let mut seed_session_manager = seed_session_manager.unwrap_or_else(|| {
        SessionManager::new(
            NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()),
            device.secret_key().to_secret_bytes(),
        )
        .snapshot()
    });
    if seed_session_manager.local_invite.is_none() {
        seed_session_manager.local_invite = Some(local_invite);
    }
    let seed_group_manager = seed_group_manager.unwrap_or_else(|| {
        NostrGroupManager::new(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())).snapshot()
    });
    ProtocolEngine::seed_storage_if_missing_for_test(
        storage.as_ref(),
        seed_session_manager,
        seed_group_manager,
    )
    .expect("seed protocol state");
    core.protocol_engine = Some(
        ProtocolEngine::load_or_create_for_local_device(
            storage,
            owner.public_key(),
            device,
        )
        .expect("protocol engine"),
    );
}

fn stable_local_invite_for_test(owner: &Keys, device: &Keys) -> Invite {
    let mut invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    invite.owner_public_key = Some(owner.public_key());
    invite
}

fn stored_message_count(core: &AppCore) -> i64 {
    let conn = core.app_store.shared();
    let count = conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    count
}

fn stored_message_expiration(core: &AppCore, chat_id: &str, message_id: &str) -> Option<u64> {
    let conn = core.app_store.shared();
    let expires_at: Option<i64> = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT expires_at_secs FROM messages WHERE chat_id = ?1 AND id = ?2",
            rusqlite::params![chat_id, message_id],
            |row| row.get(0),
        )
        .unwrap();
    expires_at.map(|secs| secs as u64)
}

fn stored_chat_ttl(core: &AppCore, chat_id: &str) -> Option<u64> {
    let conn = core.app_store.shared();
    let conn = conn.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT ttl_seconds FROM chat_message_ttls WHERE chat_id = ?1")
        .unwrap();
    let mut rows = stmt.query([chat_id]).unwrap();
    rows.next()
        .unwrap()
        .map(|row| row.get::<_, i64>(0).unwrap() as u64)
}

fn runtime_state_json(core: &AppCore, owner: &Keys, device: &Keys) -> serde_json::Value {
    let storage = crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    );
    let value = storage
        .get("appcore/protocol-engine-state-v1")
        .expect("read appcore protocol state")
        .expect("appcore protocol state exists");
    serde_json::from_str(&value).expect("runtime state json")
}

fn stored_pending_group_sender_key_message_count(
    core: &AppCore,
    owner: &Keys,
    device: &Keys,
) -> usize {
    runtime_state_json(core, owner, device)
        .get("pending_group_sender_key_messages")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

fn stored_pending_decrypted_delivery_count(core: &AppCore, owner: &Keys, device: &Keys) -> usize {
    runtime_state_json(core, owner, device)
        .get("pending_decrypted_deliveries")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

fn unknown_group_sender_key_outer_event(sender_event: &Keys) -> Event {
    use base64::Engine;

    let mut payload = Vec::new();
    payload.extend_from_slice(&7_u32.to_be_bytes());
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&[42_u8; 32]);
    let content = base64::engine::general_purpose::STANDARD.encode(payload);
    EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), content)
        .sign_with_keys(sender_event)
        .expect("unknown group sender-key outer")
}

fn delivered_texts() -> &'static std::sync::Mutex<std::collections::HashMap<usize, Vec<String>>> {
    static DELIVERED: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<usize, Vec<String>>>,
    > = std::sync::OnceLock::new();
    DELIVERED.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn runtime_key(runtime: &NdrRuntime) -> usize {
    runtime as *const NdrRuntime as usize
}

fn deliver_published_events(from: &NdrRuntime, signer: &Keys, to: &NdrRuntime) {
    for event in drain_signed_events(from, signer) {
        deliver_event_to_runtime(to, event);
    }
}

fn deliver_runtime_effects(
    from: &NdrRuntime,
    signer: &Keys,
    effects: Vec<SessionManagerEvent>,
    to: &NdrRuntime,
) {
    apply_runtime_persist_effects(from, &effects);
    let events = signed_events_from_effects(effects, signer);
    for event in &events {
        deliver_event_to_runtime(to, event.clone());
    }
    for event in events {
        from.ack_prepared_publish(&event.id.to_string())
            .expect("ack prepared publish");
        apply_runtime_persist_effects(from, &from.drain_events());
    }
}

fn accept_invite_and_deliver(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    invite: &Invite,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    acceptor
        .accept_invite(invite, Some(inviter_pubkey))
        .expect("accept invite");
    deliver_runtime_effects(acceptor, acceptor_keys, acceptor.drain_events(), inviter);
}

fn deliver_event_to_runtime(to: &NdrRuntime, event: Event) {
    to.process_received_event(event);
    let effects = to.drain_events();
    apply_runtime_persist_effects(to, &effects);
    let mut messages = Vec::new();
    for effect in effects {
        if let SessionManagerEvent::DecryptedMessage { content, .. } = effect {
            messages.push(
                serde_json::from_str::<UnsignedEvent>(&content)
                    .ok()
                    .map(|event| event.content)
                    .unwrap_or(content),
            );
        }
    }
    if !messages.is_empty() {
        delivered_texts()
            .lock()
            .unwrap()
            .entry(runtime_key(to))
            .or_default()
            .extend(messages);
    }
}

fn apply_runtime_persist_effects(_runtime: &NdrRuntime, _effects: &[SessionManagerEvent]) {
    // Runtime persistence is internal. This helper keeps existing simulated
    // relay-delivery tests readable where they previously modeled app steps.
}

fn pending_events_with_kind(core: &AppCore, kind: u32) -> Vec<Event> {
    core.pending_relay_publishes
        .values()
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .filter(|event| event.kind.as_u16() as u32 == kind)
        .collect()
}

fn complete_first_contact(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    acceptor
        .send_text(
            inviter_pubkey,
            "__ndr_first_contact_bootstrap__".to_string(),
            None,
        )
        .expect("first-contact bootstrap send");
    deliver_runtime_effects(acceptor, acceptor_keys, acceptor.drain_events(), inviter);
}

fn signed_events_from_effects(effects: Vec<SessionManagerEvent>, signer: &Keys) -> Vec<Event> {
    effects
        .into_iter()
        .filter_map(|event| match event {
            SessionManagerEvent::Publish(unsigned) if unsigned.pubkey == signer.public_key() => {
                unsigned.sign_with_keys(signer).ok()
            }
            SessionManagerEvent::PublishSigned(event) => Some(event),
            SessionManagerEvent::PublishSignedForInnerEvent { event, .. } => Some(event),
            _ => None,
        })
        .collect()
}

fn drain_signed_events(runtime: &NdrRuntime, signer: &Keys) -> Vec<Event> {
    let mut effects = runtime.drain_events();
    if effects.is_empty() {
        runtime.reload_from_storage().expect("reload runtime");
        effects.extend(runtime.drain_events());
    }
    let mut seen = HashSet::new();
    let events = signed_events_from_effects(effects, signer)
        .into_iter()
        .filter(|event| seen.insert(event.id))
        .collect::<Vec<_>>();
    for event in &events {
        runtime
            .ack_prepared_publish(&event.id.to_string())
            .expect("ack prepared publish");
        apply_runtime_persist_effects(runtime, &runtime.drain_events());
    }
    events
}

fn serializable_key_pair_for_test(keys: &Keys) -> nostr_double_ratchet::SerializableKeyPair {
    nostr_double_ratchet::SerializableKeyPair {
        public_key: NdrDevicePubkey::from_bytes(keys.public_key().to_bytes()),
        private_key: keys.secret_key().to_secret_bytes(),
    }
}

fn compact_event_payload_for_apns_test(event: &Event) -> serde_json::Value {
    let mut value = serde_json::to_value(event).expect("event json");
    if let Some(object) = value.as_object_mut() {
        let header_tags = object
            .get("tags")
            .and_then(|tags| tags.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|tag| {
                tag.as_array()
                    .and_then(|items| items.first())
                    .and_then(|name| name.as_str())
                    == Some("header")
            })
            .collect();
        object.insert("tags".to_string(), serde_json::Value::Array(header_tags));
    }
    value
}

fn drain_text_messages(runtime: &NdrRuntime) -> Vec<String> {
    delivered_texts()
        .lock()
        .unwrap()
        .remove(&runtime_key(runtime))
        .unwrap_or_default()
}

/// End-to-end round-trip: upload a real image to the hashtree network and
/// verify the same bytes can be read back via the same path the iOS shell
/// uses. Marked `ignore` because it depends on external network reachability.
/// Run manually with: cargo test profile_picture_hashtree_roundtrip --ignored -- --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn profile_picture_hashtree_roundtrip() {
    let owner = Keys::generate();
    let secret_hex = owner.secret_key().to_secret_hex();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ios/UITests/Fixtures/cat.jpg");
    let url = super::attachment_upload::upload_profile_picture_to_hashtree(&secret_hex, &path)
        .await
        .expect("upload");
    let nhash = url.strip_prefix("htree://").expect("htree:// prefix");
    let b64 = super::attachment_upload::download_hashtree_attachment_base64(nhash)
        .await
        .expect("download bytes");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("b64 decode");
    let original = std::fs::read(&path).expect("read original");
    assert_eq!(bytes, original, "downloaded bytes match original");
}
