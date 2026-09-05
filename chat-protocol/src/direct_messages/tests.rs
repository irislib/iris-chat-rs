use super::*;
use crate::{invite_url, parse_invite_event, DeviceEntry};
use nostr::Kind;

fn publish_events(commands: Vec<DirectMessageCommand>) -> Vec<Event> {
    commands
        .into_iter()
        .filter_map(|command| match command {
            DirectMessageCommand::Publish(event) => Some(event),
            DirectMessageCommand::Subscribe { .. } => None,
        })
        .collect()
}

fn publish_kinds(commands: &[DirectMessageCommand]) -> Vec<Kind> {
    commands
        .iter()
        .filter_map(|command| match command {
            DirectMessageCommand::Publish(event) => Some(event.kind),
            DirectMessageCommand::Subscribe { .. } => None,
        })
        .collect()
}

fn route_wrapped_invite_url(invite: &Invite) -> String {
    let raw = invite_url(invite, "https://chat.iris.to").expect("invite url");
    let Some((_, fragment)) = raw.split_once('#') else {
        return raw;
    };
    let payload = fragment.trim_start_matches('/');
    if payload.starts_with("invite/") {
        raw
    } else {
        format!("https://chat.iris.to/#/invite/{payload}")
    }
}

#[test]
fn chat_snapshots_select_one_latest_message_with_the_thread_tie_break() {
    let service = DirectMessageService::memory();
    let peer = Keys::generate().public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let empty = Keys::generate().public_key().to_hex();
    service.ensure_thread(&peer, 20);
    service.ensure_thread(&other, 10);
    service.ensure_thread(&empty, 30);
    for (chat_id, id, body, at) in [
        (&peer, "z", "latest", 20),
        (&peer, "a", "same second", 20),
        (&peer, "m", "middle", 20),
        (&peer, "zz", "older", 19),
        (&other, "z", "other chat", 10),
    ] {
        service.insert_message(
            chat_id,
            id,
            body,
            false,
            at,
            DirectMessageDelivery::Received,
            None,
        );
    }

    let chats = service.chats();
    assert_eq!(chats.len(), 3, "one snapshot per conversation");
    assert_eq!(chats[0].chat_id, empty);
    assert_eq!(chats[0].last_message_at, 30);
    assert_eq!(chats[0].last_message_preview, "");
    assert_eq!(chats[1].chat_id, peer);
    assert_eq!(chats[1].last_message_at, 20);
    assert_eq!(chats[1].last_message_preview, "latest");
    assert_eq!(chats[2].chat_id, other);
    assert_eq!(chats[2].last_message_preview, "other chat");

    let thread = service.thread(&peer).expect("thread");
    assert_eq!(thread.chat, chats[1]);
    assert_eq!(
        thread.messages.last().expect("latest message").body,
        "latest"
    );
}

#[test]
fn accepts_route_wrapped_invite_and_sends_direct_message() {
    let inviter_keys = Keys::generate();
    let accepter_keys = Keys::generate();
    let mut inviter =
        DirectMessageService::memory_for_local_device(inviter_keys.public_key(), &inviter_keys);
    let mut accepter =
        DirectMessageService::memory_for_local_device(accepter_keys.public_key(), &accepter_keys);
    let invite_event = inviter
        .local_invite_event(&inviter_keys)
        .expect("local invite event");
    let invite = parse_invite_event(&invite_event).expect("invite event");
    let invite_url = route_wrapped_invite_url(&invite);

    let (thread, accept_commands) = accepter
        .accept_invite(&invite_url, &accepter_keys)
        .expect("accept invite");
    assert_eq!(thread.chat.chat_id, inviter_keys.public_key().to_hex());
    let accept_kinds = publish_kinds(&accept_commands);
    assert!(accept_kinds.contains(&Kind::from(INVITE_RESPONSE_KIND as u16)));
    assert!(accept_kinds.contains(&Kind::from(MESSAGE_EVENT_KIND as u16)));

    for event in publish_events(accept_commands) {
        inviter.process_event(event, &inviter_keys);
    }

    let send_commands = accepter
        .send_message(
            &inviter_keys.public_key().to_hex(),
            "hello from invite accepter",
            &accepter_keys,
        )
        .expect("send message");
    assert!(publish_kinds(&send_commands).contains(&Kind::from(MESSAGE_EVENT_KIND as u16)));

    for event in publish_events(send_commands) {
        inviter.process_event(event, &inviter_keys);
    }

    let inviter_thread = inviter
        .thread(&accepter_keys.public_key().to_hex())
        .expect("inviter thread");
    assert_eq!(inviter_thread.messages.len(), 1);
    assert_eq!(
        inviter_thread.messages[0].body,
        "hello from invite accepter"
    );
    assert!(!inviter_thread.messages[0].is_outgoing);
    assert_eq!(
        inviter_thread.messages[0].delivery,
        DirectMessageDelivery::Received
    );
}

#[test]
fn claimed_owner_invite_retries_after_ordinary_app_keys_ingestion() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let accepter_keys = Keys::generate();
    let mut accepter =
        DirectMessageService::memory_for_local_device(accepter_keys.public_key(), &accepter_keys);
    let mut invite = Invite::create_new(
        inviter_device.public_key(),
        Some(inviter_device.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(inviter_owner.public_key());
    invite.purpose = Some("private".to_string());
    let invite_url = route_wrapped_invite_url(&invite);

    let pending = accepter
        .accept_invite_with_status(&invite_url, &accepter_keys)
        .expect("pending acceptance");
    let commands = match pending {
        DirectInviteAcceptanceOutcome::PendingOwnerRoster {
            owner_pubkey,
            device_pubkey,
            commands,
        } => {
            assert_eq!(owner_pubkey, inviter_owner.public_key().to_hex());
            assert_eq!(device_pubkey, inviter_device.public_key().to_hex());
            commands
        }
        DirectInviteAcceptanceOutcome::Accepted { .. } => {
            panic!("owner claim must wait for AppKeys")
        }
    };
    let owner_hex = inviter_owner.public_key().to_hex();
    assert!(commands.iter().any(|command| {
        matches!(
            command, DirectMessageCommand::Subscribe { filters, durable: true, .. }
                if filters.iter().any(|filter| {
                    serde_json::to_string(filter)
                        .is_ok_and(|json| json.contains(&owner_hex))
                })
        )
    }));
    assert!(accepter
        .chats()
        .iter()
        .all(|chat| chat.chat_id != owner_hex));

    let roster = AppKeys::new(vec![DeviceEntry::new(inviter_device.public_key(), 10)])
        .get_event_at(inviter_owner.public_key(), 10)
        .sign_with_keys(&inviter_owner)
        .expect("signed inviter AppKeys");
    let completion = accepter.process_event(roster.clone(), &accepter_keys);

    assert!(accepter
        .chats()
        .iter()
        .all(|chat| chat.chat_id != owner_hex));
    assert!(publish_kinds(&completion).is_empty());
    let accepted = accepter
        .accept_invite_with_status(&invite_url, &accepter_keys)
        .expect("authorized retry");
    assert!(matches!(
        accepted,
        DirectInviteAcceptanceOutcome::Accepted { .. }
    ));
}
