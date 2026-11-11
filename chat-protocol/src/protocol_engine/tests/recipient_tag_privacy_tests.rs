//! Message events name no recipient. A relay that sees `p` on kind 1060 can
//! link every delivery, and every group fanout burst, to the receiving devices.

use super::*;
use std::cell::Cell;

thread_local! {
    static LEGACY_RECIPIENT_TAGS: Cell<bool> = const { Cell::new(false) };
}

/// While alive, engines on this thread seal message events the way clients did
/// before the recipient tag was removed. Only for receive-compatibility tests.
pub(crate) struct LegacyRecipientTags(());

impl LegacyRecipientTags {
    pub(crate) fn sender() -> Self {
        LEGACY_RECIPIENT_TAGS.with(|flag| flag.set(true));
        Self(())
    }
}

impl Drop for LegacyRecipientTags {
    fn drop(&mut self) {
        LEGACY_RECIPIENT_TAGS.with(|flag| flag.set(false));
    }
}

pub(crate) fn legacy_recipient_tags() -> bool {
    LEGACY_RECIPIENT_TAGS.with(Cell::get)
}

pub(crate) fn legacy_recipient_message_event(
    envelope: &MessageEnvelope,
    recipient: NdrDevicePubkey,
) -> anyhow::Result<Event> {
    let event = nostr_double_ratchet::message_event(envelope)?;
    Ok(nostr::EventBuilder::new(event.kind, event.content.clone())
        .tags(event.tags.iter().cloned())
        .tag(nostr::Tag::public_key(public_device(recipient)?))
        .custom_created_at(event.created_at)
        .sign_with_keys(&Keys::new(nostr::SecretKey::from_slice(
            &envelope.signer_secret_key,
        )?))?)
}

/// Every key a relay could use to tie a message event to an account or device.
struct Transcript {
    identities: Vec<PublicKey>,
    messages: usize,
    pairwise_targets: BTreeSet<PublicKey>,
}

impl Transcript {
    fn new(identities: impl IntoIterator<Item = PublicKey>) -> Self {
        Self {
            identities: identities.into_iter().collect(),
            messages: 0,
            pairwise_targets: BTreeSet::new(),
        }
    }

    fn inspect(&mut self, label: &str, effects: &[ProtocolEffect]) {
        for ProtocolEffect::Publish(publish) in effects {
            let event = &publish.event;
            let wire = serde_json::to_string(event).unwrap();
            assert!(event.verify().is_ok(), "{label}: unsigned publication");
            match event.kind.as_u16() as u32 {
                MESSAGE_EVENT_KIND => {
                    let names = event
                        .tags
                        .iter()
                        .map(|tag| tag.as_slice()[0].as_str())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        names,
                        ["v", "header"],
                        "{label}: a message event carries the version and header only"
                    );
                    assert_eq!(event.tags.iter().next().unwrap().as_slice(), ["v", "2"]);
                    for identity in &self.identities {
                        assert!(
                            !wire.contains(&identity.to_hex()),
                            "{label}: message event names an account, device or invite key"
                        );
                    }
                    assert!(parse_message_event(event).unwrap().recipient.is_none());
                    match publish.carrier {
                        ProtocolCarrier::Pairwise { device } => {
                            assert!(publish.is_publishable(), "{label}");
                            self.pairwise_targets.insert(device);
                            self.messages += 1;
                        }
                        ProtocolCarrier::SharedGroup => assert!(
                            !publish.is_publishable(),
                            "{label}: shared group ciphertext offered to a carrier"
                        ),
                        ProtocolCarrier::Unmarked => {
                            panic!("{label}: message event without provenance")
                        }
                    }
                }
                INVITE_RESPONSE_KIND => {
                    assert_eq!(
                        event.tags.public_keys().count(),
                        1,
                        "{label}: an invite response targets the invite key only"
                    );
                    assert_eq!(publish.carrier, ProtocolCarrier::Unmarked);
                    assert!(publish.is_publishable());
                }
                kind => panic!("{label}: unexpected private publication kind {kind}"),
            }
        }
    }
}

#[test]
fn message_events_never_name_a_recipient_on_any_send_path() {
    let mut f = remote_send_fixture();
    observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    let sibling_device = Keys::generate();
    let mut sibling = test_engine(&f.owner, &sibling_device);
    let roster = signed_app_keys(
        &f.owner,
        &[f.device.public_key(), sibling_device.public_key()],
        2,
    );
    for engine in [&mut f.sender, &mut f.ready, &mut f.late, &mut sibling] {
        engine.ingest_app_keys_event(&roster).unwrap();
    }
    observe_sibling_invite(&mut f.sender, &sibling, &sibling_device);
    let invite_key = |engine: &ProtocolEngine| engine.local_invite_response_pubkey().unwrap();
    let mut transcript = Transcript::new([
        f.owner.public_key(),
        f.device.public_key(),
        sibling_device.public_key(),
        f.peer_owner.public_key(),
        f.ready_device.public_key(),
        f.late_device.public_key(),
        invite_key(&f.sender),
        invite_key(&f.ready),
        invite_key(&f.late),
        invite_key(&sibling),
    ]);
    let peer = f.peer_owner.public_key();
    let owner = f.owner.public_key();

    // First contact: invite responses plus the first ratchet messages, with a
    // copy for the sender's own linked device.
    let first = f
        .sender
        .send_direct_text(peer, &peer.to_hex(), "first contact", None, UnixSeconds(10))
        .unwrap();
    transcript.inspect("first contact", &first.effects);
    assert_eq!(
        transcript.pairwise_targets,
        BTreeSet::from([
            f.ready_device.public_key(),
            f.late_device.public_key(),
            sibling_device.public_key(),
        ]),
        "provenance, not the wire, names each delivery target"
    );
    let deliver = |receiver: &mut ProtocolEngine, device: &Keys, effects: &[ProtocolEffect]| {
        let effects = effects
            .iter()
            .filter(|ProtocolEffect::Publish(publish)| {
                publish.carrier.pairwise_device() == Some(device.public_key())
                    || publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND
            })
            .cloned()
            .collect();
        decrypt_own_sync_effects(receiver, effects)
    };
    assert_eq!(
        deliver(&mut f.ready, &f.ready_device, &first.effects).len(),
        1
    );
    assert_eq!(
        deliver(&mut f.late, &f.late_device, &first.effects).len(),
        1
    );
    assert_eq!(
        deliver(&mut sibling, &sibling_device, &first.effects).len(),
        1
    );

    // An established session in both directions.
    let reply = f
        .ready
        .send_direct_text(owner, &owner.to_hex(), "reply", None, UnixSeconds(11))
        .unwrap();
    transcript.inspect("reply", &reply.effects);
    assert_eq!(deliver(&mut f.sender, &f.device, &reply.effects).len(), 1);

    // A call offer travels in the same pairwise carrier, to the peer only.
    let offer =
        nostr::EventBuilder::new(nostr::Kind::Custom(21_112), "{\"type\":\"offer\"}").build(owner);
    let offered = f
        .sender
        .send_direct_unsigned_event_to_peer_only(peer, &peer.to_hex(), offer, UnixSeconds(12))
        .unwrap();
    transcript.inspect("call offer", &offered.effects);
    assert_eq!(
        deliver(&mut f.ready, &f.ready_device, &offered.effects).len(),
        1
    );

    // Linked-device sync.
    let synced = f
        .sender
        .send_local_sibling_unsigned_event(
            peer,
            &peer.to_hex(),
            own_seen_rumor(&f.owner),
            UnixSeconds(13),
        )
        .unwrap();
    transcript.inspect("linked device sync", &synced.effects);
    assert_eq!(
        deliver(&mut sibling, &sibling_device, &synced.effects).len(),
        1
    );

    // Group control, shared sender-key ciphertext and its pairwise carriers.
    let created = f
        .sender
        .create_group("no recipient tags".into(), vec![peer], unix_now())
        .unwrap();
    transcript.inspect("group creation", &created.effects);
    let group_id = created.snapshot.unwrap().group_id;
    let before = transcript.messages;
    let sent = f
        .sender
        .send_group_payload(
            &group_id,
            b"group body".to_vec(),
            Some("group-inner".into()),
        )
        .unwrap();
    transcript.inspect("group message", &sent.effects);
    assert!(transcript.messages > before);
    assert!(
        sent.effects
            .iter()
            .all(|ProtocolEffect::Publish(publish)| publish.is_publishable()),
        "shared group ciphertext must stay out of the publication batch"
    );
    transcript.inspect("group replay", &f.sender.pending_group_publish_effects());
}

#[test]
fn invite_acceptance_bootstrap_message_names_no_recipient() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut accepter = test_engine(&owner, &device);
    let mut inviter = test_engine(&peer_owner, &peer_device);
    for engine in [&mut accepter, &mut inviter] {
        engine
            .ingest_app_keys_event(&signed_app_keys(&owner, &[device.public_key()], 1))
            .unwrap();
        engine
            .ingest_app_keys_event(&signed_app_keys(
                &peer_owner,
                &[peer_device.public_key()],
                1,
            ))
            .unwrap();
    }
    let invite = inviter.local_invite().unwrap();
    let ProtocolAcceptInviteOutcome::Accepted(accepted) = accepter
        .accept_invite(&invite, Some(peer_owner.public_key()))
        .unwrap()
    else {
        panic!("invite was not accepted");
    };
    let mut transcript = Transcript::new([
        owner.public_key(),
        device.public_key(),
        peer_owner.public_key(),
        peer_device.public_key(),
        inviter.local_invite_response_pubkey().unwrap(),
    ]);
    transcript.inspect("invite bootstrap", &accepted.effects);
    assert_eq!(transcript.messages, 1);
    assert_eq!(
        transcript.pairwise_targets,
        BTreeSet::from([peer_device.public_key()])
    );
    // The inviter completes the handshake and reads the bootstrap message.
    assert_eq!(
        decrypt_own_sync_effects(&mut inviter, accepted.effects).len(),
        1
    );
}

#[test]
fn recipient_tagged_envelopes_from_older_clients_still_decrypt_and_route() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    let sent = {
        let _older_client = LegacyRecipientTags::sender();
        f.sender
            .send_direct_text(
                peer,
                &peer.to_hex(),
                "from an old client",
                None,
                UnixSeconds(10),
            )
            .unwrap()
    };
    let legacy = sent
        .effects
        .iter()
        .map(|ProtocolEffect::Publish(publish)| publish)
        .find(|publish| publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap()
        .clone();
    assert_eq!(
        legacy.event.tags.public_keys().copied().collect::<Vec<_>>(),
        [f.ready_device.public_key()],
        "fixture must reproduce the old wire shape"
    );
    assert!(
        !legacy.is_publishable(),
        "an upgraded client reads the old shape but never publishes it"
    );
    // Another device of the same account drops it without queueing a retry.
    assert!(f.late.message_targets_another_device(&legacy.event));
    assert!(f
        .late
        .process_direct_message_event(&legacy.event)
        .unwrap()
        .is_none());
    assert!(f.late.pending_inbound.is_empty());
    // The named device reads it as before.
    assert!(!f.ready.message_targets_another_device(&legacy.event));
    let messages = decrypt_own_sync_effects(&mut f.ready, sent.effects);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, f.owner.public_key());

    // The upgraded device answers without a tag; the old session keeps working.
    let owner = f.owner.public_key();
    let reply = f
        .ready
        .send_direct_text(
            owner,
            &owner.to_hex(),
            "from the new client",
            None,
            UnixSeconds(11),
        )
        .unwrap();
    let mut transcript = Transcript::new([
        f.owner.public_key(),
        f.device.public_key(),
        f.peer_owner.public_key(),
        f.ready_device.public_key(),
    ]);
    transcript.inspect("reply to an old client", &reply.effects);
    assert_eq!(
        decrypt_own_sync_effects(&mut f.sender, reply.effects).len(),
        1
    );
}

#[test]
fn recipient_tagged_ready_publications_from_an_older_version_are_retired() {
    let mut fixture = remote_send_fixture();
    let peer = fixture.peer_owner.public_key();
    let group_id = fixture
        .sender
        .create_group("upgrade".into(), vec![peer], unix_now())
        .unwrap()
        .snapshot
        .unwrap()
        .group_id;
    let sent = {
        let _older_version = LegacyRecipientTags::sender();
        fixture
            .sender
            .send_group_payload(
                &group_id,
                b"sealed before upgrade".to_vec(),
                Some("old".into()),
            )
            .unwrap()
    };
    assert!(sent.effects.iter().any(|ProtocolEffect::Publish(publish)| {
        publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
            && publish.event.tags.public_keys().next().is_some()
    }));
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        fixture.store,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert!(
        restored
            .pending_group_publish_effects()
            .iter()
            .all(
                |ProtocolEffect::Publish(publish)| publish.event.kind.as_u16() as u32
                    != MESSAGE_EVENT_KIND
            ),
        "recipient-tagged envelopes must not be offered to the outbox again"
    );
    restored
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
        .unwrap();
    assert!(restored
        .ready_group_publications
        .values()
        .all(
            |pending| pending.publish.event.tags.public_keys().next().is_none()
                || pending.publish.event.kind.as_u16() as u32 != MESSAGE_EVENT_KIND
        ));
}

#[test]
fn message_authors_are_ranked_live_keys_first_and_missed_message_keys_last() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(peer, &peer.to_hex(), "seed", None, UnixSeconds(10))
        .unwrap();
    let (device, mut state) = f
        .sender
        .session_manager
        .snapshot()
        .users
        .into_iter()
        .flat_map(|user| user.devices)
        .find_map(|device| Some((device.device_pubkey, device.active_session?)))
        .expect("the first delivery established a session");
    // A key that only still has missed messages from an older ratchet period.
    let missed = Keys::generate().public_key();
    state.skipped_keys.insert(
        ndr_device(missed),
        nostr_double_ratchet::SkippedKeysEntry {
            message_keys: [(0, [7; 32])].into_iter().collect(),
            header_keys: Vec::new(),
        },
    );
    f.sender.session_manager.import_session_state(
        ndr_owner(peer),
        device,
        state,
        NdrUnixSeconds(10),
    );
    f.sender.invalidate_known_message_author_cache();

    let ranked = f.sender.message_author_pubkeys_by_priority(|_| true);
    let unordered = f.sender.message_author_pubkeys_filtered(|_| true);
    assert!(ranked.len() > 1);
    assert_eq!(
        ranked.iter().copied().collect::<BTreeSet<_>>(),
        unordered.iter().copied().collect::<BTreeSet<_>>(),
        "ranking must not add or lose a subscribed author"
    );
    assert_eq!(
        ranked.last(),
        Some(&missed),
        "a relay limit must drop missed-message keys before live ratchet keys"
    );
    assert!(f
        .sender
        .message_author_pubkeys_by_priority(|owner| owner != peer)
        .iter()
        .all(|author| *author != missed));
}
