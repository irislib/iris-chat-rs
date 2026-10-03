use super::direct_chat_capability::{resolve_current_app_keys, CurrentAppKeysResolution};

fn app_keys_event(owner: &Keys, devices: &[&Keys], created_at_secs: u64) -> Event {
    AppKeys::new(
        devices
            .iter()
            .map(|device| DeviceEntry::new(device.public_key(), created_at_secs))
            .collect(),
    )
    .get_event_at(owner.public_key(), created_at_secs)
    .sign_with_keys(owner)
    .expect("sign AppKeys")
}

fn prime_capability_check(core: &mut AppCore, owner: PublicKey) -> (u64, u64) {
    core.direct_chat_capability_runtime.next_token = core
        .direct_chat_capability_runtime
        .next_token
        .wrapping_add(1)
        .max(1);
    let token = core.direct_chat_capability_runtime.next_token;
    core.direct_chat_capability_runtime.current = Some(DirectChatCapabilityCheck {
        token,
        owner_pubkey_hex: owner.to_hex(),
        state: DirectChatCapabilityCheckState::Checking,
    });
    (core.direct_chat_capability_runtime.generation, token)
}

fn direct_capability_session_core(receive_only: bool) -> (AppCore, Keys, Keys) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-session", &owner, &device);
    let engine = core.protocol_engine.as_mut().unwrap();
    observe_peer_appkeys_for_test(engine, &peer, &[peer_device.public_key()], 1);
    let mut session = established_peer_session_state_for_test(&peer_device, &device);
    if receive_only {
        session.our_current_nostr_key = None;
    }
    engine
        .import_session_state(
            peer.public_key(),
            Some(peer_device.public_key().to_hex()),
            session,
            UnixSeconds(2),
        )
        .unwrap();
    assert!(!core.app_keys.contains_key(&peer.public_key().to_hex()));
    assert!(core.logged_in.as_ref().unwrap().relay_urls.is_empty());
    (core, peer, peer_device)
}

#[test]
fn direct_capability_existing_authenticated_session_opens_offline_without_lookup() {
    let (mut core, peer, _) = direct_capability_session_core(false);
    let chat_id = peer.public_key().to_hex();
    prime_capability_check(&mut core, peer.public_key());
    core.open_chat(&chat_id);

    assert_eq!(
        core.state.current_chat.as_ref().unwrap().direct_chat_capability,
        Some(DirectChatCapabilityState::Available),
        "the protocol's authenticated session is sufficient even without the app-key UI cache"
    );
    assert!(core.direct_chat_capability_runtime.current.is_none());
    assert!(!core.request_direct_chat_capability_check(&chat_id, true));
}

#[test]
fn direct_capability_unknown_peer_only_checks_when_lookup_starts() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("direct-capability-unknown", &owner, &device);
    core.ensure_thread_record(&chat_id, 1).messages.push(test_chat_message(
        &chat_id, "old-message", "old history is not a session", 1, false,
    ));
    assert_eq!(core.chat_capability(&chat_id, &ChatKind::Direct), None);

    core.logged_in.as_mut().unwrap().relay_urls = vec!["ws://127.0.0.1:9".parse().unwrap()];
    assert!(core.request_direct_chat_capability_check(&chat_id, false));
    assert_eq!(
        core.chat_capability(&chat_id, &ChatKind::Direct),
        Some(DirectChatCapabilityState::Checking)
    );
}

#[test]
fn direct_capability_receive_only_session_does_not_skip_lookup() {
    let (mut core, peer, _) = direct_capability_session_core(true);
    let chat_id = peer.public_key().to_hex();
    assert!(core.request_direct_chat_capability_check(&chat_id, false));
    assert_eq!(
        core.chat_capability(&chat_id, &ChatKind::Direct),
        Some(DirectChatCapabilityState::CheckFailed),
        "a receive-only session still needs discovery; no message servers means retry"
    );
}

#[test]
fn direct_capability_existing_session_cannot_override_authoritative_revocation() {
    let (mut core, peer, _) = direct_capability_session_core(false);
    let chat_id = peer.public_key().to_hex();
    assert_eq!(
        core.chat_capability(&chat_id, &ChatKind::Direct),
        Some(DirectChatCapabilityState::Available)
    );
    core.handle_relay_event(app_keys_event(&peer, &[], 3));
    assert_eq!(
        core.chat_capability(&chat_id, &ChatKind::Direct),
        Some(DirectChatCapabilityState::Unavailable)
    );
    core.app_keys.remove(&chat_id);
    core.reset_direct_chat_capability_runtime();
    assert_ne!(
        core.chat_capability(&chat_id, &ChatKind::Direct),
        Some(DirectChatCapabilityState::Available),
        "removing the UI cache cannot revive protocol-revoked sessions"
    );
}

#[test]
fn direct_capability_optimistic_page_does_not_invent_a_lookup() {
    let (mut core, peer, _) = direct_capability_session_core(false);
    let chat_id = peer.public_key().to_hex();
    core.ensure_thread_record(&chat_id, 1);
    core.rebuild_state();
    assert!(core.state.current_chat.is_none());
    let shared = core.app_store.shared();
    let provisional = chat_snapshot_from_state_and_db(&core.state, Some(&shared), &chat_id, 80)
        .expect("local chat page");
    assert_eq!(provisional.direct_chat_capability, None);

    core.open_chat(&chat_id);
    let resolved = chat_snapshot_from_state_and_db(&core.state, Some(&shared), &chat_id, 80)
        .expect("resolved chat page");
    assert_eq!(resolved.direct_chat_capability, Some(DirectChatCapabilityState::Available));
}

#[test]
fn direct_capability_requires_a_verified_unique_current_nonempty_head() {
    let owner = Keys::generate();
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let imposter = Keys::generate();
    let now = unix_now().get();
    let current = app_keys_event(&owner, &[&device_a], now);
    let older_empty = app_keys_event(&owner, &[], now.saturating_sub(1));
    let future_empty = app_keys_event(&owner, &[], now.saturating_add(301));
    let forged = AppKeys::new(vec![DeviceEntry::new(device_b.public_key(), now)])
        .get_event_at(owner.public_key(), now.saturating_add(1))
        .sign_with_keys(&imposter)
        .expect("build forged AppKeys");

    assert!(matches!(
        resolve_current_app_keys(
            vec![older_empty, future_empty, forged, current],
            owner.public_key(),
            now,
        ),
        CurrentAppKeysResolution::Found {
            has_devices: true,
            ..
        }
    ));

    let conflicting_a = app_keys_event(&owner, &[&device_a], now);
    let conflicting_b = app_keys_event(&owner, &[&device_b], now);
    assert_eq!(
        resolve_current_app_keys(
            vec![conflicting_a, conflicting_b],
            owner.public_key(),
            now,
        ),
        CurrentAppKeysResolution::Ambiguous
    );

    let newer_empty = app_keys_event(&owner, &[], now.saturating_add(1));
    assert!(matches!(
        resolve_current_app_keys(
            vec![app_keys_event(&owner, &[&device_a], now), newer_empty],
            owner.public_key(),
            now.saturating_add(1),
        ),
        CurrentAppKeysResolution::Found {
            has_devices: false,
            ..
        }
    ));
}

#[test]
fn direct_capability_completion_unlocks_only_nonempty_app_keys() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-completion", &local_owner, &local_device);
    let peer_hex = peer_owner.public_key().to_hex();
    let now = unix_now().get();

    let (generation, token) = prime_capability_check(&mut core, peer_owner.public_key());
    core.ensure_thread_record(&peer_hex, now);
    core.active_chat_id = Some(peer_hex.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: peer_hex.clone(),
    }];
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .and_then(|chat| chat.direct_chat_capability.clone()),
        Some(DirectChatCapabilityState::Checking)
    );
    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer_hex,
        Ok(vec![app_keys_event(
            &peer_owner,
            &[&peer_device],
            now,
        )]),
    );
    assert_eq!(
        core.direct_chat_capability_state(&peer_hex),
        Some(DirectChatCapabilityState::Available)
    );
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .and_then(|chat| chat.direct_chat_capability.clone()),
        Some(DirectChatCapabilityState::Available)
    );

    let (generation, token) = prime_capability_check(&mut core, peer_owner.public_key());
    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer_hex,
        Ok(vec![app_keys_event(
            &peer_owner,
            &[],
            now.saturating_add(1),
        )]),
    );
    assert_eq!(
        core.direct_chat_capability_state(&peer_hex),
        Some(DirectChatCapabilityState::Unavailable)
    );

    let (generation, token) = prime_capability_check(&mut core, peer_owner.public_key());
    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer_hex,
        Err("offline".to_string()),
    );
    assert_eq!(
        core.direct_chat_capability_state(&peer_hex),
        Some(DirectChatCapabilityState::CheckFailed)
    );
}

#[test]
fn direct_capability_completion_is_invalidated_on_account_reset() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-reset", &local_owner, &local_device);
    let peer_hex = peer_owner.public_key().to_hex();
    let (generation, token) = prime_capability_check(&mut core, peer_owner.public_key());

    core.reset_direct_chat_capability_runtime();
    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer_hex,
        Ok(vec![app_keys_event(
            &peer_owner,
            &[&peer_device],
            unix_now().get(),
        )]),
    );

    assert!(!core.app_keys.contains_key(&peer_hex));
    assert_eq!(
        core.direct_chat_capability_state(&peer_hex),
        None
    );
}

#[test]
fn direct_capability_completion_is_latest_chat_wins() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let old_peer = Keys::generate();
    let old_device = Keys::generate();
    let current_peer = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-latest", &local_owner, &local_device);
    let (generation, old_token) = prime_capability_check(&mut core, old_peer.public_key());
    let _ = prime_capability_check(&mut core, current_peer.public_key());

    core.handle_direct_chat_capability_fetch_finished(
        generation,
        old_token,
        &old_peer.public_key().to_hex(),
        Ok(vec![app_keys_event(
            &old_peer,
            &[&old_device],
            unix_now().get(),
        )]),
    );

    assert!(!core.app_keys.contains_key(&old_peer.public_key().to_hex()));
    assert_eq!(
        core.direct_chat_capability_state(&current_peer.public_key().to_hex()),
        Some(DirectChatCapabilityState::Checking)
    );
}

#[test]
fn direct_capability_unlocks_when_subscription_finds_devices_during_check() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-subscription", &owner, &device);
    prime_capability_check(&mut core, peer.public_key());

    core.handle_relay_event(app_keys_event(&peer, &[&peer_device], unix_now().get()));

    assert_eq!(
        core.direct_chat_capability_state(&peer.public_key().to_hex()),
        Some(DirectChatCapabilityState::Available),
        "verified devices arriving through a subscription must unlock the composer"
    );
}

#[test]
fn direct_capability_completion_with_stale_devices_finishes_unavailable() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-stale-devices", &owner, &device);
    let now = unix_now().get();
    core.handle_relay_event(app_keys_event(&peer, &[], now));
    let (generation, token) = prime_capability_check(&mut core, peer.public_key());

    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer.public_key().to_hex(),
        Ok(vec![app_keys_event(&peer, &[&peer_device], now - 1)]),
    );

    assert_eq!(
        core.direct_chat_capability_state(&peer.public_key().to_hex()),
        Some(DirectChatCapabilityState::Unavailable),
        "a completed lookup must not keep checking or revive revoked devices"
    );
}

#[test]
fn direct_capability_completion_restores_seen_devices_missing_from_app_cache() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-seen-devices", &owner, &device);
    let event = app_keys_event(&peer, &[&peer_device], unix_now().get());
    core.handle_relay_event(event.clone());
    core.app_keys.remove(&peer.public_key().to_hex());
    let (generation, token) = prime_capability_check(&mut core, peer.public_key());

    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer.public_key().to_hex(),
        Ok(vec![event]),
    );

    assert_eq!(
        core.direct_chat_capability_state(&peer.public_key().to_hex()),
        Some(DirectChatCapabilityState::Available),
        "event deduplication must not prevent rebuilding the device cache"
    );
}

#[test]
fn direct_capability_resumes_after_completion_was_dropped_while_suspended() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-capability-suspend", &owner, &device);
    let peer_hex = peer.public_key().to_hex();
    core.ensure_thread_record(&peer_hex, unix_now().get());
    core.active_chat_id = Some(peer_hex.clone());
    core.screen_stack = vec![Screen::Chat { chat_id: peer_hex.clone() }];
    let (generation, token) = prime_capability_check(&mut core, peer.public_key());

    core.prepare_for_suspend();
    core.handle_internal(InternalEvent::DirectChatCapabilityFetchFinished {
        generation,
        token,
        owner_pubkey_hex: peer_hex.clone(),
        result: Ok(vec![app_keys_event(&peer, &[&peer_device], unix_now().get())]),
    });
    assert!(!core.app_keys.contains_key(&peer_hex));
    core.handle_app_foregrounded();

    assert_eq!(
        core.direct_chat_capability_state(&peer_hex),
        Some(DirectChatCapabilityState::CheckFailed),
        "resuming without message servers must expose retry, not wait on a dropped completion"
    );
    core.handle_direct_chat_capability_fetch_finished(
        generation,
        token,
        &peer_hex,
        Ok(vec![app_keys_event(&peer, &[&peer_device], unix_now().get())]),
    );
    assert!(!core.app_keys.contains_key(&peer_hex), "pre-suspend results must remain invalidated");
}
