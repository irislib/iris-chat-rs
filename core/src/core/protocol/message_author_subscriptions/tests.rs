use super::*;
use crate::core::publish_helpers::{authenticated_relay, identity_receiver};

/// strfry's default frame size; a larger request is refused whole.
const RELAY_FRAME_BYTES: usize = 131_072;
/// strfry refuses a filter with more items than this.
const RELAY_FILTER_ITEMS: usize = 2_047;

fn sorted_authors(count: usize) -> Vec<PublicKey> {
    let mut authors = (0..count)
        .map(|_| Keys::generate().public_key())
        .collect::<Vec<_>>();
    authors.sort();
    authors
}

fn plan_with_message_authors(authors: &[PublicKey]) -> ProtocolSubscriptionPlan {
    ProtocolSubscriptionPlan {
        runtime_subscriptions: vec![PROTOCOL_SUBSCRIPTION_ID.to_string()],
        roster_authors: Vec::new(),
        invite_authors: Vec::new(),
        message_authors: authors.iter().map(PublicKey::to_hex).collect(),
        group_roster_group_ids: Vec::new(),
        group_roster_authors: Vec::new(),
        group_sender_key_authors: Vec::new(),
        invite_response_recipient: Some(Keys::generate().public_key().to_hex()),
    }
}

fn chunk_authors(filter: &Filter) -> Vec<PublicKey> {
    filter.authors.iter().flatten().copied().collect()
}

#[test]
fn thousands_of_message_authors_become_several_requests_within_relay_limits() {
    let authors = sorted_authors(3_500);
    let plan = plan_with_message_authors(&authors);
    for (chunk_size, frame_bytes, filters) in [
        (
            RELAY_MESSAGE_AUTHOR_CHUNK_SIZE,
            RELAY_FRAME_BYTES,
            build_protocol_subscription_filters(&plan),
        ),
        (
            MESH_MESSAGE_AUTHOR_CHUNK_SIZE,
            nostr_pubsub_fips::FIPS_NOSTR_PUBSUB_MAX_FRAME_BYTES,
            build_protocol_subscription_filters_with_chunk_size(
                &plan,
                MESH_MESSAGE_AUTHOR_CHUNK_SIZE,
            ),
        ),
    ] {
        let (shared, chunks) = split_message_author_filters(filters);
        assert!(
            chunks.len() >= authors.len().div_ceil(chunk_size) && chunks.len() > 1,
            "a few thousand authors need several subscriptions, got {}",
            chunks.len()
        );
        let mut seen = HashSet::new();
        for chunk in &chunks {
            let members = chunk_authors(chunk);
            assert!(!members.is_empty());
            assert!(members.len() <= chunk_size, "chunk of {}", members.len());
            assert!(members.len() <= RELAY_FILTER_ITEMS);
            // The limit applies to the whole request, so each chunk is alone.
            let request = serde_json::json!(["REQ", "ndr-protocol-messages-999", chunk]);
            assert!(
                request.to_string().len() < frame_bytes,
                "one chunk must fit one frame"
            );
            for author in members {
                assert!(seen.insert(author), "an author is requested twice");
            }
        }
        assert_eq!(seen, authors.iter().copied().collect::<HashSet<_>>());
        // Nothing else rides in a message-author request, and the shared
        // request does not grow with the number of message authors.
        assert!(!shared.is_empty());
        assert!(shared.iter().all(|filter| !filter
            .kinds
            .as_ref()
            .is_some_and(|kinds| kinds.contains(&Kind::from(MESSAGE_EVENT_KIND as u16)))));
        assert!(serde_json::to_string(&shared).unwrap().len() < 1_024);
    }
    // A small author set stays in one subscription.
    let small = plan_with_message_authors(&authors[..40]);
    let (_, chunks) = split_message_author_filters(build_protocol_subscription_filters(&small));
    assert_eq!(chunks.len(), 1);
    // Catch-up asks per chunk too: an oversized request loses all of it.
    let mut core_plan = plan;
    core_plan.invite_response_recipient = None;
    let catch_up = message_author_subscription_filters(
        pubkeys_from_hexes(&core_plan.message_authors),
        RELAY_MESSAGE_AUTHOR_CHUNK_SIZE,
    );
    assert!(catch_up
        .iter()
        .all(|filter| serde_json::to_string(filter).unwrap().len() < RELAY_FRAME_BYTES));
}

fn message_requests(rows: &[(usize, serde_json::Value)]) -> Vec<&serde_json::Value> {
    rows.iter()
        .map(|(_, frame)| frame)
        .filter(|frame| {
            frame[0] == "REQ"
                && frame[1]
                    .as_str()
                    .is_some_and(|id| id.starts_with(MESSAGE_AUTHOR_SUBSCRIPTION_PREFIX))
        })
        .collect()
}

/// Frames reach the relay's log a moment after the client sent them. Wait
/// for the expected one, then a little longer so any extra frame shows too.
fn settled_wire(
    wire: &std::sync::Mutex<Vec<(usize, serde_json::Value)>>,
    from: usize,
    expect_message_request: bool,
) -> Vec<(usize, serde_json::Value)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let rows = wire.lock().unwrap().clone();
        if !expect_message_request || !message_requests(&rows[from..]).is_empty() {
            std::thread::sleep(Duration::from_millis(200));
            return wire.lock().unwrap().clone();
        }
        assert!(
            Instant::now() < deadline,
            "no message request reached the relay"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn reconcile_and_settle(
    core: &mut AppCore,
    messages: &flume::Receiver<CoreMsg>,
    plan: &ProtocolSubscriptionPlan,
) {
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    core.reconcile_protocol_subscriptions("message-author-chunks", false);
    let deadline = Instant::now() + Duration::from_secs(10);
    while core.protocol_subscription_runtime.applied_plan.as_ref() != Some(plan)
        || core.protocol_subscription_runtime.refresh_in_flight
    {
        assert!(Instant::now() < deadline, "subscription plan did not apply");
        match messages.recv_timeout(Duration::from_millis(20)) {
            Ok(message) => core.handle_message(message),
            Err(_) => continue,
        };
    }
}

#[test]
fn adding_a_message_author_requests_only_its_own_chunk_again() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, _dir) = crate::core::tests::logged_in_test_core_with_updates(
        "message-author-chunks",
        &owner,
        &device,
    );
    let (relay, wire, client) = core.runtime.block_on(async {
        let (relay, wire) = authenticated_relay(device.public_key(), false).await;
        let client = identity_receiver(&device, &relay, &wire).await;
        (relay, wire, client)
    });
    core.logged_in.as_mut().unwrap().client = client;
    core.logged_in.as_mut().unwrap().relay_urls = vec![relay.clone()];
    core.preferences.nostr_relay_urls = vec![relay.to_string()];
    core.relay_status
        .by_url
        .insert(relay.to_string(), RelayStatus::Connected);
    let (sender, messages) = flume::unbounded();
    core.core_sender = sender.clone();
    core.priority_sender = sender;

    // Today's layout is key-ordered, so only an author that sorts last is
    // sure to touch a single chunk. With the library's key-selected layout
    // this holds for any author; see the ignored test below.
    let mut authors = sorted_authors(2_501);
    let late = authors.pop().unwrap();
    let before = plan_with_message_authors(&authors);
    reconcile_and_settle(&mut core, &messages, &before);

    let rows = settled_wire(&wire, 0, true);
    let first = message_requests(&rows);
    assert!(first.len() > 1, "expected several message subscriptions");
    let mut requested = HashSet::new();
    let mut ids = HashSet::new();
    for frame in &first {
        assert_eq!(
            frame.as_array().unwrap().len(),
            3,
            "a message-author request carries exactly one filter"
        );
        assert!(frame.to_string().len() < RELAY_FRAME_BYTES);
        assert!(ids.insert(frame[1].as_str().unwrap().to_string()));
        let members = frame[2]["authors"].as_array().unwrap();
        assert!(members.len() <= RELAY_MESSAGE_AUTHOR_CHUNK_SIZE);
        assert_eq!(frame[2]["kinds"], serde_json::json!([MESSAGE_EVENT_KIND]));
        assert!(frame[2].get("#p").is_none());
        requested.extend(
            members
                .iter()
                .map(|author| author.as_str().unwrap().to_string()),
        );
    }
    assert_eq!(
        requested,
        authors
            .iter()
            .map(PublicKey::to_hex)
            .collect::<HashSet<_>>()
    );
    // The shared request holds the rest and no message authors.
    let shared = rows
        .iter()
        .map(|(_, frame)| frame)
        .filter(|frame| frame[0] == "REQ" && frame[1] == PROTOCOL_SUBSCRIPTION_ID)
        .collect::<Vec<_>>();
    assert_eq!(shared.len(), 1);
    assert!(!shared[0].to_string().contains(&authors[0].to_hex()));

    // An unchanged plan sends nothing again.
    let sent = rows.len();
    core.protocol_subscription_runtime.applied_plan = None;
    reconcile_and_settle(&mut core, &messages, &before);
    let rows = settled_wire(&wire, sent, false);
    assert!(message_requests(&rows[sent..]).is_empty());

    // One more author: one subscription is replaced, the others stay as sent.
    let sent = rows.len();
    authors.push(late);
    let mut after = plan_with_message_authors(&authors);
    after.invite_response_recipient = before.invite_response_recipient.clone();
    reconcile_and_settle(&mut core, &messages, &after);
    let rows = settled_wire(&wire, sent, true);
    let again = message_requests(&rows[sent..]);
    assert_eq!(
        again.len(),
        1,
        "one new author must re-request one chunk, not all of them"
    );
    assert!(again[0][2]["authors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|author| author == &serde_json::json!(late.to_hex())));
    let closed = rows[sent..]
        .iter()
        .filter(|(_, frame)| frame[0] == "CLOSE")
        .map(|(_, frame)| frame[1].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(
        closed.len() <= 1 && closed.iter().all(|id| id == again[0][1].as_str().unwrap()),
        "only the replaced chunk may be closed: {closed:?}"
    );
    assert!(
        !rows[sent..]
            .iter()
            .any(|(_, frame)| frame[0] == "REQ" && frame[1] == PROTOCOL_SUBSCRIPTION_ID),
        "the shared request did not change"
    );
}

/// The owner's rule: an author lands in the chunk its key selects, so one
/// ratchet step replaces one subscription. That layout lives in
/// `nostr_double_ratchet::build_message_author_subscription_filters`, which is
/// not in the pinned 0.0.177. Enable when the pin moves and
/// `message_author_subscription_filters` calls it.
#[test]
#[ignore = "needs build_message_author_subscription_filters from nostr-double-ratchet > 0.0.177"]
fn adding_any_message_author_changes_exactly_one_chunk() {
    let mut authors = sorted_authors(3_501);
    // Not the last key: an author anywhere in key order.
    let added = authors.remove(authors.len() / 3);
    let chunks = |authors: &[PublicKey]| {
        message_author_subscription_filters(
            authors.iter().copied(),
            RELAY_MESSAGE_AUTHOR_CHUNK_SIZE,
        )
    };
    let before = chunks(&authors);
    authors.push(added);
    let after = chunks(&authors);
    assert!(before.len() > 1);
    assert!(after
        .iter()
        .all(|chunk| chunk_authors(chunk).len() <= RELAY_MESSAGE_AUTHOR_CHUNK_SIZE));
    let changed = after
        .iter()
        .filter(|chunk| !before.contains(chunk))
        .collect::<Vec<_>>();
    assert_eq!(changed.len(), 1, "one author must change one chunk");
    assert!(chunk_authors(changed[0]).contains(&added));
}
