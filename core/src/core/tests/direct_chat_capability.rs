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
        DirectChatCapabilityState::Available
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
        DirectChatCapabilityState::Unavailable
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
        DirectChatCapabilityState::CheckFailed
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
        DirectChatCapabilityState::Checking
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
        DirectChatCapabilityState::Checking
    );
}
