use super::*;

fn nearby_link(device: PublicKey) -> crate::updates::FipsNearbyLinkSnapshot {
    crate::updates::FipsNearbyLinkSnapshot {
        device_pubkey_hex: device.to_hex(),
        transport_type: "ble".into(),
        transport_addr: None,
    }
}

/// Envelopes the sender queued for exactly `device`.
fn queued_envelopes(core: &AppCore, device: PublicKey) -> Vec<Event> {
    sorted_pending_events_for_test(core)
        .into_iter()
        .filter(|event| queued_for_device(core, event, device))
        .collect()
}

fn has_message(core: &AppCore, chat_id: &str, body: &str) -> bool {
    core.threads[chat_id]
        .messages
        .iter()
        .any(|message| message.body == body)
}

#[test]
fn group_message_reaches_a_nearby_member_inside_its_own_ratchet_envelope() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let near = farm.devices[1].keys.public_key();
    let far = farm.devices[2].keys.public_key();
    let body = "offline over the nearby link";

    // The first message also distributes the sender key; get that out of the way.
    farm.send(0, &chat_id, "warm up");
    farm.pump();

    // No member in range: the group data stays one shared record, for message
    // servers only, and nothing is addressed to a device.
    let idle = queued_envelopes(&farm.devices[0].core, near).len();
    farm.send(0, &chat_id, "nobody in range");
    assert_eq!(queued_envelopes(&farm.devices[0].core, near).len(), idle);
    farm.pump();

    // Member 1 is on a live nearby link; member 2 is not.
    farm.devices[0]
        .core
        .update_fips_connection_links(vec![nearby_link(near)]);
    let before = queued_envelopes(&farm.devices[0].core, near);
    let far_before = queued_envelopes(&farm.devices[0].core, far).len();
    farm.send(0, &chat_id, body);
    let sender = &farm.devices[0].core;
    let wakes = queued_envelopes(sender, near)
        .into_iter()
        .filter(|event| !before.contains(event))
        .collect::<Vec<_>>();
    assert_eq!(wakes.len(), 1, "one envelope for the nearby device");
    assert_eq!(
        queued_envelopes(sender, far).len(),
        far_before,
        "a member out of range gets no nearby copy"
    );
    let wake = &wakes[0];

    // Wire shape: an ordinary ratchet message event. It names no recipient,
    // is signed by a session key, and is offered only to the linked device.
    assert_eq!(wake.kind.as_u16() as u32, MESSAGE_EVENT_KIND);
    assert!(wake.tags.public_keys().next().is_none());
    let wire = serde_json::to_string(wake).unwrap();
    for identity in [
        farm.owners[0].public_key(),
        farm.devices[0].keys.public_key(),
        farm.owners[1].public_key(),
        near,
    ] {
        assert!(
            !wire.contains(&identity.to_hex()),
            "the nearby envelope names no identity"
        );
    }
    assert!(!wire.contains(&group_id), "the group is not named either");
    assert!(sender.private_publication_allowed(wake));
    assert!(sender.carrier_publication_allowed(wake));
    assert_eq!(
        sender.private_nearby_targets(wake),
        Some(std::collections::BTreeSet::from([fips_peer_from_hex(
            &near.to_hex()
        )
        .unwrap()
        .npub()])),
        "sent to exactly the linked member device"
    );

    // The shared group record itself still never goes to a nearby carrier.
    let shared = sorted_pending_events_for_test(sender)
        .into_iter()
        .find(|event| {
            sender
                .pending_relay_publishes
                .get(&event.id.to_hex())
                .is_some_and(|pending| pending.label == GROUP_DATA_PUBLISH_LABEL)
        })
        .expect("shared group record stays queued for message servers");
    assert!(!sender.carrier_publication_allowed(&shared));

    // Offline delivery: the receiver gets only the nearby envelope.
    let wake = wake.clone();
    let receiver = &mut farm.devices[1].core;
    assert!(!has_message(receiver, &chat_id, body));
    assert!(receiver.handle_relay_event_with_channel(wake, "nearby"));
    assert!(has_message(receiver, &chat_id, body));

    // Back online, the shared record arrives too and is not shown twice.
    farm.pump();
    for device in &farm.devices[1..] {
        assert_eq!(
            device.core.threads[&chat_id]
                .messages
                .iter()
                .filter(|message| message.body == body)
                .count(),
            1
        );
    }
}

#[test]
fn queued_group_message_is_sealed_when_a_member_comes_into_range() {
    let (mut farm, _group_id, chat_id) = private_group_wake_farm();
    let near = farm.devices[1].keys.public_key();
    let body = "queued before the link came up";
    farm.send(0, &chat_id, "warm up");
    farm.pump();
    let before = queued_envelopes(&farm.devices[0].core, near);
    farm.send(0, &chat_id, body);
    assert_eq!(queued_envelopes(&farm.devices[0].core, near), before);

    farm.devices[0]
        .core
        .update_fips_connection_links(vec![nearby_link(near)]);
    let wakes = queued_envelopes(&farm.devices[0].core, near)
        .into_iter()
        .filter(|event| !before.contains(event))
        .collect::<Vec<_>>();
    assert_eq!(wakes.len(), 1);
    let sealed = queued_envelopes(&farm.devices[0].core, near).len();

    // A link flap does not seal the same message again.
    farm.devices[0]
        .core
        .update_fips_connection_links(Vec::new());
    farm.devices[0]
        .core
        .update_fips_connection_links(vec![nearby_link(near)]);
    assert_eq!(queued_envelopes(&farm.devices[0].core, near).len(), sealed);

    let receiver = &mut farm.devices[1].core;
    assert!(receiver.handle_relay_event_with_channel(wakes[0].clone(), "nearby"));
    assert!(has_message(receiver, &chat_id, body));
}
