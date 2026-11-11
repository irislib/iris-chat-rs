//! Message events name no recipient. A relay that sees a recipient on kind
//! 1060 can link every delivery, and every group fanout burst, to the devices
//! receiving it. These tests read what real sends hand to relays and carriers.

use super::*;
use nostr::util::JsonUtil;
use std::collections::BTreeSet;

/// Every account, device and public invite key a relay could link to a person.
fn farm_identities(farm: &GroupScaleFarm) -> Vec<String> {
    let mut identities = farm
        .owners
        .iter()
        .map(|owner| owner.public_key().to_hex())
        .collect::<Vec<_>>();
    for device in &farm.devices {
        identities.push(device.keys.public_key().to_hex());
        identities.push(
            device
                .core
                .protocol_engine
                .as_ref()
                .unwrap()
                .local_invite_response_pubkey()
                .unwrap()
                .to_hex(),
        );
    }
    identities
}

fn assert_message_event_names_nobody(label: &str, event: &Event, identities: &[String]) {
    assert_eq!(event.kind.as_u16() as u32, MESSAGE_EVENT_KIND);
    let tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(tags.len(), 2, "{label}: unexpected tags {tags:?}");
    assert_eq!(tags[0], ["v", "2"], "{label}");
    assert_eq!(tags[1][0], "header", "{label}");
    let wire = event.as_json();
    for identity in identities {
        assert!(
            !wire.contains(identity),
            "{label}: message event names an account, device or invite key"
        );
    }
    assert!(event.verify().is_ok(), "{label}");
}

fn bodies(core: &AppCore, chat_id: &str) -> Vec<String> {
    core.threads
        .get(chat_id)
        .map(|thread| {
            thread
                .messages
                .iter()
                .map(|message| message.body.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn assert_no_recipient_subscription(farm: &GroupScaleFarm) {
    for (index, device) in farm.devices.iter().enumerate() {
        let now = unix_now();
        let live = device
            .core
            .compute_protocol_subscription_plan()
            .map(|plan| build_protocol_subscription_filters(&plan))
            .unwrap_or_default();
        for filter in live.iter().chain(&device.core.recent_protocol_filters(now)) {
            let asks_for_messages = filter
                .kinds
                .as_ref()
                .is_some_and(|kinds| kinds.contains(&Kind::from(MESSAGE_EVENT_KIND as u16)));
            assert!(
                !asks_for_messages || filter.generic_tags.is_empty(),
                "device {index} asks a relay for messages by recipient: {}",
                serde_json::to_string(filter).unwrap()
            );
            assert!(
                !asks_for_messages
                    || !serde_json::to_string(filter)
                        .unwrap()
                        .contains(&device.keys.public_key().to_hex()),
                "device {index} names itself in a message subscription"
            );
        }
    }
}

/// Reads one posted call wake, the way the notification service receives it.
fn capture_call_wake(core: &mut AppCore) -> std::thread::JoinHandle<Event> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    core.preferences.mobile_push_server_url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map(|value| value.trim().parse().unwrap())
                .unwrap();
            if bytes.len() >= end + 4 + length {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
                break serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    })
}

#[test]
fn published_message_events_name_no_recipient_on_any_send_path() {
    // Alice has two linked devices (0 and 1); Bob (2) and Carol (3) have one.
    let mut farm = GroupScaleFarm::new(&[2, 1, 1]);
    farm.pump();
    let identities = farm_identities(&farm);
    let alice = farm.owners[0].public_key().to_hex();
    let bob = farm.owners[1].public_key().to_hex();
    let carol = farm.owners[2].public_key().to_hex();
    let alice_sibling = farm.devices[1].keys.public_key();
    let bob_device = farm.devices[2].keys.clone();

    // First contact, with a copy for Alice's linked device.
    farm.devices[0].core.handle_action(AppAction::CreateChat {
        peer_input: bob.clone(),
    });
    farm.pump();
    farm.send(0, &bob, "first contact");
    {
        // Before any relay accepts them: each queued envelope names nobody,
        // and its delivery target exists only in the local queue.
        let sender = &farm.devices[0].core;
        let queued = pending_events_with_kind(sender, MESSAGE_EVENT_KIND);
        assert!(!queued.is_empty(), "first contact queued no message event");
        let mut targets = BTreeSet::new();
        for event in &queued {
            assert_message_event_names_nobody("queued first contact", event, &identities);
            let device = sender
                .queued_pairwise_device(event)
                .expect("a queued message event has a recorded target device");
            assert!(sender.carrier_publication_allowed(event));
            assert_eq!(
                sender.private_nearby_targets(event),
                Some(BTreeSet::from([test_fips_peer_for_key(device).npub()])),
                "nearby copies go only to the recorded device"
            );
            targets.insert(device);
        }
        assert!(targets.contains(&bob_device.public_key()));
        assert!(targets.contains(&alice_sibling));
    }
    farm.pump();
    assert!(
        bodies(&farm.devices[2].core, &alice).contains(&"first contact".to_string()),
        "Bob must read a first-contact message that names no recipient: {:?}",
        farm.devices[2]
            .core
            .debug_log
            .iter()
            .map(|entry| format!("{}:{}", entry.category, entry.detail))
            .collect::<Vec<_>>()
    );
    assert!(
        bodies(&farm.devices[1].core, &bob).contains(&"first contact".to_string()),
        "Alice's linked device must receive its copy"
    );

    // An established session, in the other direction.
    farm.send(2, &alice, "reply");
    farm.pump();
    for device in 0..2 {
        assert!(
            bodies(&farm.devices[device].core, &bob).contains(&"reply".to_string()),
            "Alice's device {device} must read the reply"
        );
    }

    // A group: control messages, sender-key distribution and pairwise carriers.
    farm.devices[0]
        .core
        .create_group("No recipient tags", &[bob.clone(), carol.clone()]);
    farm.pump();
    let group_id = farm.devices[0].core.groups.keys().next().unwrap().clone();
    let chat_id = group_chat_id(&group_id);
    for device in &mut farm.devices {
        assert!(device.core.groups.contains_key(&group_id));
        device
            .core
            .handle_action(AppAction::SetMessageRequestAccepted {
                chat_id: chat_id.clone(),
            });
    }
    farm.send(0, &chat_id, "group body");
    let shared_authors = farm.devices[0]
        .core
        .protocol_engine
        .as_ref()
        .unwrap()
        .group_sender_event_pubkeys_for_group(&group_id);
    farm.pump();
    farm.retry();
    for device in 1..4 {
        assert!(
            bodies(&farm.devices[device].core, &chat_id).contains(&"group body".to_string()),
            "device {device} must read the group message"
        );
    }

    // A call offer, and the wake that rings Bob's device.
    let queued_before = pending_events_with_kind(&farm.devices[0].core, MESSAGE_EVENT_KIND)
        .into_iter()
        .map(|event| event.id)
        .collect::<HashSet<_>>();
    let silent_peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let wake = {
        let caller = &mut farm.devices[0].core;
        call_test_peer(caller, &farm.owners[1], &bob_device);
        caller.reconcile_calls_udp_for_test(
            "127.0.0.1:0".parse().unwrap(),
            silent_peer.local_addr().unwrap(),
            &test_fips_peer(&bob_device).npub(),
        );
        let wake = capture_call_wake(caller);
        caller.handle_action(AppAction::StartCall {
            chat_id: bob.clone(),
            video: false,
        });
        wake
    };
    let offers = farm.devices[0]
        .core
        .pending_relay_publishes
        .values()
        .filter(|pending| pending.chat_id.as_deref() == Some(bob.as_str()))
        .map(|pending| serde_json::from_str::<Event>(&pending.event_json).unwrap())
        .filter(|event| {
            event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND && !queued_before.contains(&event.id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        offers.len(),
        1,
        "one call-offer carrier for Bob's one device"
    );
    assert_message_event_names_nobody("call offer", &offers[0], &identities);
    let wake = wake.join().unwrap();
    assert_eq!(wake.kind.as_u16(), 21_111);
    assert_eq!(
        wake.tags.public_keys().copied().collect::<Vec<_>>(),
        [bob_device.public_key()],
        "the wake rings the device recorded for the offer, not one read from it"
    );
    let content: serde_json::Value = serde_json::from_str(&wake.content).unwrap();
    let inner = content["events"].as_array().unwrap();
    assert_eq!(inner.len(), 1);
    let inner: Event = serde_json::from_value(inner[0].clone()).unwrap();
    assert_eq!(inner, offers[0]);
    assert_message_event_names_nobody("call wake payload", &inner, &identities);
    farm.pump();

    // Everything any device handed to the message servers.
    let messages = farm
        .history
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .collect::<Vec<_>>();
    assert!(
        messages.len() >= 8,
        "expected direct, linked-device, group and call envelopes, saw {}",
        messages.len()
    );
    for event in &messages {
        assert_message_event_names_nobody("relay transcript", event, &identities);
        assert!(
            !shared_authors.contains(&event.pubkey),
            "shared group ciphertext reached a message server"
        );
    }
    assert!(messages.iter().any(|event| event.id == offers[0].id));
    // Only an invite response names a target, and only the public invite key.
    for event in farm
        .history
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
    {
        assert_eq!(event.tags.public_keys().count(), 1);
    }
    assert_no_recipient_subscription(&farm);
}

fn test_fips_peer_for_key(device: PublicKey) -> fips_core::PeerIdentity {
    fips_core::PeerIdentity::from_npub(&device.to_bech32().unwrap()).unwrap()
}

/// A peer running a version from before the recipient tag was removed, or the
/// current one, opening a session with `receiver` through its public invite.
struct RawSender {
    owner: Keys,
    session: nostr_double_ratchet::Session,
    response: Event,
}

impl RawSender {
    fn accept(receiver: &AppCore) -> Self {
        let owner = Keys::generate();
        let invite = receiver
            .protocol_engine
            .as_ref()
            .unwrap()
            .local_invite()
            .unwrap();
        let (session, response) = invite
            .accept_with_owner(
                owner.public_key(),
                owner.secret_key().to_secret_bytes(),
                Some(owner.public_key().to_hex()),
                Some(owner.public_key()),
            )
            .unwrap();
        Self {
            owner,
            session,
            response: invite_response_event(&response).unwrap(),
        }
    }

    /// `named` reproduces the old wire shape: one `p` tag for that device.
    fn message(&mut self, body: &str, named: Option<PublicKey>) -> Event {
        let now = unix_now().get();
        let (content, _) = runtime_rumor_json(
            self.owner.public_key(),
            CHAT_MESSAGE_KIND,
            body,
            now,
            Vec::new(),
        );
        let plan = self
            .session
            .plan_send(content.as_bytes(), NdrUnixSeconds(now))
            .unwrap();
        let envelope = self.session.apply_send(plan).envelope;
        let event = message_event(&envelope).unwrap();
        let Some(named) = named else {
            return event;
        };
        EventBuilder::new(event.kind, event.content.clone())
            .tags(event.tags.iter().cloned())
            .tag(nostr::Tag::public_key(named))
            .custom_created_at(event.created_at)
            .sign_with_keys(&Keys::new(
                nostr::SecretKey::from_slice(&envelope.signer_secret_key).unwrap(),
            ))
            .unwrap()
    }
}

fn pending_inbound(core: &AppCore) -> usize {
    core.protocol_engine
        .as_ref()
        .unwrap()
        .debug_snapshot()
        .pending_inbound_count
}

fn call_wake_for(device: PublicKey, message: &Event) -> Event {
    EventBuilder::new(
        Kind::from(21_111),
        serde_json::json!({"type":"call-wake","v":3,"events":[message]}).to_string(),
    )
    .tag(nostr::Tag::public_key(device))
    .sign_with_keys(&Keys::generate())
    .unwrap()
}

#[test]
fn messages_that_overtake_their_invite_response_are_held_then_read() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("untagged-before-response", &owner, &device);
    let mut nearby = RawSender::accept(&core);
    let mut waking = RawSender::accept(&core);
    let mut older = RawSender::accept(&core);
    let untagged = nearby.message("untagged nearby copy before its handshake", None);
    let woken = waking.message("untagged wake payload before its handshake", None);
    let tagged = older.message(
        "recipient-tagged before its handshake",
        Some(device.public_key()),
    );
    assert!(untagged.tags.public_keys().next().is_none());
    assert_eq!(
        tagged.tags.public_keys().copied().collect::<Vec<_>>(),
        [device.public_key()]
    );

    // No sender is known yet: the invite responses have not arrived. A
    // message server cannot return such an event (the app subscribes by
    // sender key only), so one that shows up there is somebody else's.
    core.handle_relay_event(untagged.clone());
    assert_eq!(
        pending_inbound(&core),
        0,
        "unrelated public authors must not become pending work"
    );
    // A nearby peer's direct copy and a call wake naming this device are
    // addressed here, so their messages wait for the handshake.
    assert!(core.accepts_private_nearby_event(&untagged));
    assert!(core.handle_directed_private_event(untagged.clone(), "FIPS nearby"));
    assert_eq!(
        pending_inbound(&core),
        1,
        "a message that names nobody must wait for its invite response"
    );
    core.receive_call_push(&call_wake_for(device.public_key(), &woken));
    assert_eq!(pending_inbound(&core), 2);
    assert!(core.accepts_private_nearby_event(&tagged));
    assert!(core.handle_directed_private_event(tagged.clone(), "FIPS nearby"));
    assert_eq!(pending_inbound(&core), 3);

    for (sender, body) in [
        (&nearby, "untagged nearby copy before its handshake"),
        (&waking, "untagged wake payload before its handshake"),
        (&older, "recipient-tagged before its handshake"),
    ] {
        core.handle_relay_event(sender.response.clone());
        assert!(
            bodies(&core, &sender.owner.public_key().to_hex()).contains(&body.to_string()),
            "{body}: held message was lost"
        );
    }
    assert_eq!(pending_inbound(&core), 0);

    // Established sessions keep reading both shapes from message servers.
    core.handle_relay_event(nearby.message("untagged later", None));
    core.handle_relay_event(older.message("recipient-tagged later", Some(device.public_key())));
    assert!(
        bodies(&core, &nearby.owner.public_key().to_hex()).contains(&"untagged later".to_string())
    );
    assert!(bodies(&core, &older.owner.public_key().to_hex())
        .contains(&"recipient-tagged later".to_string()));
}

#[test]
fn messages_naming_another_device_are_dropped_and_the_wait_is_bounded() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("foreign-recipient", &owner, &device);
    let mut older = RawSender::accept(&core);
    let foreign = older.message("for another device", Some(Keys::generate().public_key()));
    assert!(
        !core.accepts_private_nearby_event(&foreign),
        "a nearby copy naming another device is refused at the door"
    );
    core.handle_directed_private_event(foreign.clone(), "FIPS nearby");
    core.receive_call_push(&call_wake_for(device.public_key(), &foreign));
    core.handle_relay_event(foreign);
    core.handle_relay_event(older.response.clone());
    assert_eq!(pending_inbound(&core), 0);
    assert!(bodies(&core, &older.owner.public_key().to_hex()).is_empty());

    // Unknown senders cannot fill the wait queue without limit.
    let other_owner = Keys::generate();
    let other_device = Keys::generate();
    let other = logged_in_test_core("unrelated-receiver", &other_owner, &other_device);
    for index in 0..300 {
        let mut stranger = RawSender::accept(&other);
        core.handle_directed_private_event(
            stranger.message(&format!("noise {index}"), None),
            "FIPS nearby",
        );
    }
    assert_eq!(
        pending_inbound(&core),
        256,
        "held unknown-sender messages must stay bounded"
    );
}

#[test]
fn envelopes_queued_by_the_previous_version_are_quarantined_not_republished() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("recipient-tag-upgrade", &owner, &device);
    // One chat per envelope: startup restores only each chat's latest message.
    let chats = ["tagged", "unrecorded", "current"]
        .map(|inner| (inner, Keys::generate().public_key().to_hex()));
    let chat_for = |inner: &str| {
        chats
            .iter()
            .find(|(name, _)| *name == inner)
            .map(|(_, chat)| chat.clone())
            .unwrap()
    };
    let recipient_device = Keys::generate().public_key();
    let envelope = |body: &str, named: bool| {
        let mut tags = vec![
            nostr::Tag::parse(["v", "2"]).unwrap(),
            nostr::Tag::parse(["header", "encrypted-header"]).unwrap(),
        ];
        if named {
            tags.push(nostr::Tag::public_key(recipient_device));
        }
        EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), body)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    };
    // The previous version's pairwise envelope, an envelope with no record of
    // what it is (shared group ciphertext looks the same), and a current one.
    let tagged = envelope("queued by the previous version", true);
    let unrecorded = envelope("no delivery record", false);
    let current = envelope("queued by this version", false);
    let saved = |event: &Event, inner: &str, pairwise: Option<PublicKey>| PendingRelayPublish {
        owner_pubkey_hex: owner.public_key().to_hex(),
        event_id: event.id.to_hex(),
        label: APPCORE_PROTOCOL_LABEL.into(),
        event_json: event.as_json(),
        inner_event_id: Some(inner.into()),
        chat_id: Some(chat_for(inner)),
        created_at_secs: event.created_at.as_secs(),
        attempt_count: 3,
        last_error: None,
        pairwise_device_hex: pairwise.map(|device| device.to_hex()),
    };
    for (event, inner, pairwise) in [
        // A stored record must not rescue an envelope that names its recipient.
        (&tagged, "tagged", Some(recipient_device)),
        (&unrecorded, "unrecorded", None),
        (&current, "current", Some(recipient_device)),
    ] {
        core.push_outgoing_message_with_id(
            inner.into(),
            &chat_for(inner),
            format!("history {inner}"),
            unix_now().get(),
            None,
            DeliveryState::Queued,
        );
        core.app_store
            .upsert_pending_relay_publish(&saved(event, inner, pairwise))
            .unwrap();
    }
    core.persist_best_effort();
    core.start_session(owner.public_key(), Some(owner.clone()), device, true, true)
        .unwrap();
    core.run_session_startup_follow_up();
    core.retry_pending_relay_publishes("recipient-tag-upgrade");

    assert!(core
        .pending_relay_publishes
        .contains_key(&current.id.to_hex()));
    let retained = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    for (event, inner) in [(&tagged, "tagged"), (&unrecorded, "unrecorded")] {
        assert!(
            !core
                .pending_relay_publishes
                .contains_key(&event.id.to_hex()),
            "{inner}: must leave the live outbox"
        );
        // The wire shape alone refuses the tagged one. The unrecorded one has
        // the current shape, so only the missing delivery record stops it.
        assert_eq!(
            core.private_publication_allowed(event),
            inner == "unrecorded"
        );
        assert!(!core.carrier_publication_allowed(event));
        assert!(core.private_nearby_targets(event).unwrap().is_empty());
        let row = retained
            .iter()
            .find(|row| row.event_id == event.id.to_hex())
            .expect("quarantined ciphertext stays on the device");
        assert_eq!(row.event_json, event.as_json());
        assert!(row
            .last_error
            .as_deref()
            .unwrap()
            .contains("will not be resent"));
        let message = core.threads[&chat_for(inner)]
            .messages
            .iter()
            .find(|message| message.id == inner)
            .expect("quarantine keeps the message history");
        assert_eq!(message.body, format!("history {inner}"));
        assert_eq!(message.delivery, DeliveryState::Failed);
        // No carrier may be handed it again, however it is offered.
        core.emit_nearby_published_event(event);
        assert!(!core.publish_runtime_event(event.clone(), APPCORE_PROTOCOL_LABEL, None));
    }
    assert!(!core.publish_protocol_event(ProtocolPublish {
        authored_at_secs: None,
        event: tagged.clone(),
        chat_id: chat_for("tagged"),
        inner_event_id: Some("tagged".into()),
        carrier: ProtocolCarrier::Pairwise {
            device: recipient_device,
        },
    }));
    for carrier in [ProtocolCarrier::Unmarked, ProtocolCarrier::SharedGroup] {
        assert!(!core.publish_protocol_event(ProtocolPublish {
            authored_at_secs: None,
            event: unrecorded.clone(),
            chat_id: chat_for("unrecorded"),
            inner_event_id: Some("unrecorded".into()),
            carrier,
        }));
    }
    assert!(!updates.try_iter().any(|update| {
        matches!(update, AppUpdate::NearbyPublishedEvent { event_json, .. }
            if [&tagged, &unrecorded].iter().any(|event| event_json == event.as_json()))
    }));
}
