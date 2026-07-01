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

fn owner_pubkey_for_group_test(keys: &Keys) -> nostr_double_ratchet::OwnerPubkey {
    nostr_double_ratchet::OwnerPubkey::from_bytes(keys.public_key().to_bytes())
}

fn group_snapshot_for_members_test(
    group_id: &str,
    name: &str,
    creator: &Keys,
    members: &[&Keys],
    timestamp: u64,
) -> nostr_double_ratchet::group::GroupSnapshot {
    let creator_owner = owner_pubkey_for_group_test(creator);
    let mut member_owners = members
        .iter()
        .map(|member| owner_pubkey_for_group_test(member))
        .collect::<Vec<_>>();
    if !member_owners.contains(&creator_owner) {
        member_owners.push(creator_owner);
    }
    nostr_double_ratchet::group::GroupSnapshot {
        group_id: group_id.to_string(),
        protocol: nostr_double_ratchet::GroupProtocol::sender_key_v1(),
        name: name.to_string(),
        picture: None,
        about: None,
        created_by: creator_owner,
        members: member_owners,
        admins: vec![creator_owner],
        revision: 1,
        created_at: nostr_double_ratchet::UnixSeconds(timestamp),
        updated_at: nostr_double_ratchet::UnixSeconds(timestamp),
    }
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
fn group_readiness_converges_when_metadata_arrives() {
    let mut devices = sender_key_matrix_devices(2);
    let alice_device = devices.remove(0);
    let mut bob_device = devices.remove(0);
    let created = bob_device
        .engine
        .create_group(
            "phase3 metadata convergence".to_string(),
            vec![alice_device.owner.public_key()],
            UnixSeconds(50),
        )
        .expect("bob creates group for alice");
    let created_group = created.snapshot.as_ref().expect("created group snapshot");
    let group_id = created_group.group_id.clone();
    let chat_id = group_chat_id(&group_id);

    let mut core = logged_in_test_core(
        "readiness-group-converges",
        &alice_device.owner,
        &alice_device.device,
    );
    core.protocol_engine = Some(alice_device.engine);
    core.ensure_thread_record(&chat_id, 1);
    core.active_chat_id = Some(chat_id.clone());
    core.rebuild_state();

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::GroupMetadataMissing
    );
    assert!(!chat.protocol_readiness.can_send);

    let incoming = deliver_protocol_effects_to_engine(
        core.protocol_engine.as_mut().expect("alice protocol engine"),
        &created.effects,
    );
    let metadata = incoming
        .into_iter()
        .find(|event| {
            matches!(
                event,
                GroupIncomingEvent::MetadataUpdated(group) if group.group_id == group_id
            )
        })
        .expect("pairwise group metadata event");
    core.apply_group_decrypted_event(metadata);
    core.rebuild_state();

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::Ready
    );
    assert!(chat.protocol_readiness.can_send);

    let mut not_joined = created_group.clone();
    not_joined
        .members
        .retain(|owner| owner.to_string() != alice_device.owner.public_key().to_hex());
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(not_joined));
    core.rebuild_state();

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::GroupNotJoined
    );
    assert!(!chat.protocol_readiness.can_send);
}

#[test]
fn group_readiness_converges_from_member_appkeys_and_invite_events() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut core = logged_in_test_core("readiness-group-member-converges", &alice_owner, &alice_device);
    let group_id = "member_readiness_group".to_string();
    let chat_id = group_chat_id(&group_id);
    let bob_hex = bob_owner.public_key().to_hex();
    let bob_device_hex = bob_device.public_key().to_hex();
    let local_device_hex = alice_device.public_key().to_hex();
    let group = group_snapshot_for_members_test(
        &group_id,
        "Member Readiness",
        &alice_owner,
        &[&alice_owner, &bob_owner],
        70,
    );

    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group));
    core.active_chat_id = Some(chat_id.clone());
    core.rebuild_state();

    let chat = core.state.current_chat.as_ref().expect("current group chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::GroupMemberAppKeysMissing
    );
    assert!(!chat.protocol_readiness.can_send);
    let missing_appkeys_plan = core
        .compute_protocol_subscription_plan()
        .expect("missing group member AppKeys plan");
    assert!(
        missing_appkeys_plan.roster_authors.contains(&bob_hex),
        "not-ready group must subscribe to missing member AppKeys"
    );

    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: "blocked group send".to_string(),
    });
    assert_eq!(
        core.state.toast.as_deref(),
        Some("This group is not ready yet. Waiting for member app keys.")
    );
    assert!(
        core.threads
            .get(&chat_id)
            .is_some_and(|thread| thread.messages.iter().all(|message| {
                !message.is_outgoing || message.body != "blocked group send"
            })),
        "missing member AppKeys must block sends without local outgoing rows"
    );
    assert!(
        core.pending_relay_publishes.is_empty(),
        "blocked group readiness send must not produce relay publishes"
    );

    let app_keys_event = AppKeys::new(vec![DeviceEntry::new(bob_device.public_key(), 71)])
        .get_event(bob_owner.public_key())
        .sign_with_keys(&bob_owner)
        .expect("signed Bob AppKeys");
    core.handle_relay_event(app_keys_event);

    let chat = core.state.current_chat.as_ref().expect("current group chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::GroupMemberSessionMissing
    );
    assert!(!chat.protocol_readiness.can_send);
    let missing_session_plan = core
        .compute_protocol_subscription_plan()
        .expect("missing group member session plan");
    assert!(
        missing_session_plan.invite_authors.contains(&bob_device_hex),
        "known group member devices must be tracked for invite events"
    );
    assert!(
        missing_session_plan
            .message_recipients
            .contains(&local_device_hex),
        "local message-recipient bootstrap must remain active until member sessions exist"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(72), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        ndr_device_pubkey(bob_device.public_key()),
        Some(ndr_owner_pubkey(bob_owner.public_key())),
        None,
    )
    .expect("Bob invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&bob_device)
        .expect("signed Bob invite");
    core.handle_relay_event(invite_event);

    let chat = core.state.current_chat.as_ref().expect("current group chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::Ready
    );
    assert!(chat.protocol_readiness.can_send);
}

#[test]
fn group_readiness_requires_every_current_non_local_member() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let carol_owner = Keys::generate();
    let carol_device = Keys::generate();
    let mut core = logged_in_test_core("readiness-group-all-members", &alice_owner, &alice_device);
    let group_id = "all_members_readiness_group".to_string();
    let chat_id = group_chat_id(&group_id);

    observe_peer_device_invite_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &bob_owner,
        &bob_device,
        80,
    );

    let group = group_snapshot_for_members_test(
        &group_id,
        "All Members Readiness",
        &alice_owner,
        &[&alice_owner, &bob_owner, &carol_owner],
        81,
    );
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group));
    core.active_chat_id = Some(chat_id);
    core.rebuild_state();

    let chat = core.state.current_chat.as_ref().expect("current group chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::GroupMemberAppKeysMissing
    );

    let carol_app_keys_event = AppKeys::new(vec![DeviceEntry::new(carol_device.public_key(), 82)])
        .get_event(carol_owner.public_key())
        .sign_with_keys(&carol_owner)
        .expect("signed Carol AppKeys");
    core.handle_relay_event(carol_app_keys_event);

    let chat = core.state.current_chat.as_ref().expect("current group chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::GroupMemberSessionMissing
    );
    assert!(
        !chat.protocol_readiness.can_send,
        "one missing member session must block the whole current group"
    );
}

#[test]
fn ready_group_send_creates_local_row_and_signed_pending_publish() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut core = logged_in_test_core("readiness-group-ready-send", &alice_owner, &alice_device);
    let bob_hex = bob_owner.public_key().to_hex();

    observe_peer_device_invite_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &bob_owner,
        &bob_device,
        90,
    );

    core.handle_action(AppAction::CreateGroup {
        name: "Ready Send Group".to_string(),
        member_inputs: vec![bob_hex],
    });

    let chat_id = core
        .state
        .current_chat
        .as_ref()
        .expect("current group chat")
        .chat_id
        .clone();
    let chat = core.state.current_chat.as_ref().expect("current group chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::Ready
    );
    assert!(chat.protocol_readiness.can_send);

    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: "ready group send".to_string(),
    });

    let thread = core.threads.get(&chat_id).expect("group thread");
    assert!(
        thread
            .messages
            .iter()
            .any(|message| message.is_outgoing && message.body == "ready group send"),
        "ready group send must create a local outgoing row"
    );
    assert!(
        !core.pending_relay_publishes.is_empty(),
        "ready group send must preserve signed relay publish retry"
    );
    assert!(
        core.pending_relay_publishes
            .values()
            .all(|pending| serde_json::from_str::<Event>(&pending.event_json).is_ok()),
        "pending relay publishes must contain already-signed Nostr events"
    );
}

#[test]
fn group_outer_missing_sender_key_state_is_diagnostic_only() {
    use base64::Engine as _;

    let mut devices = sender_key_matrix_devices(2);
    let mut alice_device = devices.remove(0);
    let bob_device = devices.remove(0);
    let created = alice_device
        .engine
        .create_group(
            "pending diagnostic group".to_string(),
            vec![bob_device.owner.public_key()],
            UnixSeconds(100),
        )
        .expect("Alice creates group");
    let group_id = created
        .snapshot
        .as_ref()
        .expect("created group snapshot")
        .group_id
        .clone();
    let sender_key = alice_device
        .engine
        .group_manager_snapshot_for_test()
        .sender_keys
        .into_iter()
        .find(|record| record.group_id == group_id)
        .expect("Alice sender-key record");
    let sender_event_secret_key = sender_key
        .sender_event_secret_key
        .expect("Alice sender-event secret key");
    let author_secret =
        nostr::SecretKey::from_slice(&sender_event_secret_key).expect("sender-event secret");
    let author_keys = Keys::new(author_secret);
    let key_id = sender_key.latest_key_id.unwrap_or(1);
    let mut legacy_payload = Vec::new();
    legacy_payload.extend_from_slice(&key_id.to_be_bytes());
    legacy_payload.extend_from_slice(&0u32.to_be_bytes());
    legacy_payload.extend_from_slice(b"diagnostic pending ciphertext");
    let legacy_content = base64::engine::general_purpose::STANDARD.encode(legacy_payload);
    let outer_event = nostr::EventBuilder::new(
        nostr::Kind::from(MESSAGE_EVENT_KIND as u16),
        legacy_content,
    )
    .custom_created_at(nostr::Timestamp::from(101))
    .build(author_keys.public_key())
    .sign_with_keys(&author_keys)
    .expect("signed legacy sender-key event");

    let mut bob_core = logged_in_test_core(
        "readiness-group-pending-diagnostic",
        &bob_device.owner,
        &bob_device.device,
    );
    bob_core.protocol_engine = Some(bob_device.engine);
    bob_core.debug_log.clear();
    bob_core.pending_relay_publishes.clear();

    bob_core.handle_relay_event(outer_event);

    assert!(
        bob_core.debug_log.iter().any(|entry| {
            entry.category == "appcore.protocol.group.outer.pending"
                && entry.detail.contains("event_id=")
        }),
        "missing receive-side sender-key state must be logged as diagnostic pending work: {:?}",
        bob_core.debug_log
    );
    assert!(
        bob_core.pending_relay_publishes.is_empty(),
        "pending receive diagnostics must not generate relay publishes"
    );
    let debug = bob_core
        .protocol_engine
        .as_ref()
        .expect("Bob protocol engine")
        .debug_snapshot();
    assert_eq!(debug.pending_group_sender_key_retry_count, 0);
    assert_eq!(debug.pending_group_sender_key_repair_count, 0);
    assert_eq!(debug.pending_group_fanout_count, 0);
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
