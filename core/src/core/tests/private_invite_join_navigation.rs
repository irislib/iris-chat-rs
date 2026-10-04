#[test]
fn private_invite_join_opens_chat_without_a_first_message() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut inviter = isolated_invite_ui_core("join-auto-open-inviter", &owner, &device);
    inviter.handle_action(AppAction::CreatePublicInvite);
    inviter.handle_action(AppAction::PushScreen { screen: Screen::NewChat });
    assert!(matches!(inviter.screen_stack.last(), Some(Screen::NewChat)));
    let url = inviter.state.public_invite.as_ref().unwrap().url.clone();
    let mut joiner = isolated_invite_ui_core("join-auto-open-peer", &peer_owner, &peer_device);
    joiner.handle_action(AppAction::AcceptInvite { invite_input: url });
    prove_invite_owner(&mut joiner, &owner, &device, 10);
    let response = pending_events_with_kind(&joiner, INVITE_RESPONSE_KIND).remove(0);
    inviter.handle_relay_event(response);
    assert!(matches!(inviter.screen_stack.last(), Some(Screen::NewChat)),
        "Unverified owner claim must not open a chat");
    prove_invite_owner(&mut inviter, &peer_owner, &peer_device, 10);
    assert!(matches!(inviter.screen_stack.last(), Some(Screen::Chat { chat_id }) if chat_id == &peer_owner.public_key().to_hex()),
        "An authenticated invite join should open the chat without any text message");
    assert!(inviter.threads[&peer_owner.public_key().to_hex()].messages.is_empty());
    assert!(inviter.private_chat_invites.is_empty(), "Consumption must be durable before navigation");
}

#[test]
fn private_invite_join_respects_unchecked_away_and_different_invite() {
    for mode in ["unchecked", "away", "different_invite"] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let mut inviter = isolated_invite_ui_core("invite-no-focus-steal", &owner, &device);
        inviter.handle_action(AppAction::CreatePublicInvite);
        inviter.handle_action(AppAction::PushScreen { screen: Screen::NewChat });
        assert!(inviter.state.public_invite.as_ref().unwrap().open_chat_on_join);
        let url = inviter.state.public_invite.as_ref().unwrap().url.clone();
        let mut joiner = isolated_invite_ui_core("invite-no-focus-steal-peer", &peer_owner, &peer_device);
        joiner.handle_action(AppAction::AcceptInvite { invite_input: url.clone() });
        prove_invite_owner(&mut joiner, &owner, &device, 10);
        match mode {
            "unchecked" => {
                inviter.handle_action(AppAction::SetInviteOpenOnJoin { enabled: false });
                assert!(!inviter.state.public_invite.as_ref().unwrap().open_chat_on_join);
            }
            "away" => inviter.handle_action(AppAction::PushScreen { screen: Screen::Settings }),
            "different_invite" => {
                // Invite timestamps are seconds; give the newly displayed invite a distinct time.
                std::thread::sleep(Duration::from_millis(1100));
                inviter.handle_action(AppAction::CreatePublicInvite);
                assert_ne!(inviter.state.public_invite.as_ref().unwrap().url, url);
            }
            _ => unreachable!(),
        }
        let before = inviter.screen_stack.clone();
        inviter.handle_relay_event(pending_events_with_kind(&joiner, INVITE_RESPONSE_KIND).remove(0));
        prove_invite_owner(&mut inviter, &peer_owner, &peer_device, 10);
        assert!(inviter.protocol_engine.as_ref().unwrap().active_session_count_for_owner(peer_owner.public_key()) > 0);
        assert!(inviter.threads[&peer_owner.public_key().to_hex()].messages.is_empty());
        assert_eq!(inviter.screen_stack, before, "{mode}: handshake must not steal navigation");
    }
}

fn isolated_invite_ui_core(label: &str, owner: &Keys, device: &Keys) -> AppCore {
    let mut core = logged_in_test_core(label, owner, device);
    core.preferences.nearby_enabled = false;
    core.preferences.nostr_relay_urls.clear();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(address) = socket.local_addr().unwrap() else { unreachable!() };
    drop(socket);
    core.reconcile_device_sync_at_rendezvous_for_test(address);
    core
}
