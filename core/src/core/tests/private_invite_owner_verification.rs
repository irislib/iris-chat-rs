fn crafted_private_invite_response(
    invite_url: &str,
    invitee_device: &Keys,
    payload_device_id: Option<String>,
    claimed_owner: Option<PublicKey>,
) -> Event {
    let invite = super::invites::parse_public_invite_input(invite_url)
        .expect("parse private invite for response");
    let (_, envelope) = invite
        .accept_with_owner(
            invitee_device.public_key(),
            invitee_device.secret_key().to_secret_bytes(),
            payload_device_id,
            claimed_owner,
        )
        .expect("accept private invite");
    invite_response_event(&envelope).expect("build private invite response event")
}

fn create_private_invite_for_test(core: &mut AppCore) -> String {
    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::CreatePublicInvite);
    core.state
        .public_invite
        .as_ref()
        .expect("private invite")
        .url
        .clone()
}

fn active_session_device_pubkeys(core: &AppCore, owner: PublicKey) -> Vec<PublicKey> {
    let owner = NdrOwnerPubkey::from_bytes(owner.to_bytes());
    core.protocol_engine
        .as_ref()
        .expect("protocol engine")
        .session_manager_snapshot_for_test()
        .users
        .into_iter()
        .filter(|user| user.owner_pubkey == owner)
        .flat_map(|user| user.devices)
        .filter(|device| device.active_session.is_some())
        .map(|device| {
            PublicKey::from_slice(&device.device_pubkey.to_bytes()).expect("device pubkey")
        })
        .collect()
}

fn stored_session_count(core: &AppCore, owner: PublicKey) -> usize {
    let owner = NdrOwnerPubkey::from_bytes(owner.to_bytes());
    core.protocol_engine
        .as_ref()
        .expect("protocol engine")
        .session_manager_snapshot_for_test()
        .users
        .into_iter()
        .filter(|user| user.owner_pubkey == owner)
        .flat_map(|user| user.devices)
        .map(|device| usize::from(device.active_session.is_some()) + device.inactive_sessions.len())
        .sum()
}

fn forged_private_invite_url(inviter_device: &Keys, claimed_owner: PublicKey) -> String {
    let mut invite = Invite::create_new(
        inviter_device.public_key(),
        Some(inviter_device.public_key().to_hex()),
        Some(1),
    )
    .expect("create forged private invite");
    invite.owner_public_key = Some(claimed_owner);
    invite.inviter_owner_pubkey = Some(NdrOwnerPubkey::from_bytes(claimed_owner.to_bytes()));
    invite.purpose = Some("private".to_string());
    super::invites::chat_invite_url(&invite).expect("serialize forged private invite")
}

fn ingest_invite_owner_app_keys(
    core: &mut AppCore,
    owner: PublicKey,
    events: Vec<Event>,
) {
    for event in events {
        core.handle_relay_event(event);
    }
    core.retry_pending_private_invite_responses(owner);
    core.resume_pending_outgoing_invite_acceptance(owner);
}

fn prove_invite_owner(core: &mut AppCore, owner: &Keys, device: &Keys, created_at: u64) {
    let app_keys = signed_app_keys_authorization_event(owner, device.public_key(), created_at);
    ingest_invite_owner_app_keys(core, owner.public_key(), vec![app_keys]);
}

#[test]
fn private_invite_owner_claim_without_roster_waits_before_side_effects() {
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let claimed_owner = Keys::generate();
    let attacker_device = Keys::generate();
    let invite_url = forged_private_invite_url(&attacker_device, claimed_owner.public_key());
    let mut bob = logged_in_test_core(
        "private-accept-missing-owner-roster",
        &bob_owner,
        &bob_device,
    );
    bob.pending_relay_publishes.clear();

    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });

    assert!(bob.state.toast.as_deref().is_some_and(|toast| toast.contains("Verifying")));
    assert!(bob.pending_outgoing_invite_acceptance.is_some());
    assert!(!bob
        .threads
        .contains_key(&claimed_owner.public_key().to_hex()));
    assert!(bob.pending_relay_publishes.is_empty());
    assert!(active_session_device_pubkeys(&bob, claimed_owner.public_key()).is_empty());

    bob.pending_outgoing_invite_acceptance
        .as_mut()
        .expect("pending acceptance")
        .queued_at_secs = unix_now().get().saturating_sub(5 * 60);
    bob.retry_protocol_engine_pending_work("test_timeout");
    assert!(bob.pending_outgoing_invite_acceptance.is_none());
    assert!(!bob.state.busy.accepting_invite);
    assert!(bob
        .state
        .toast
        .as_deref()
        .is_some_and(|toast| toast.contains("Reopen")));
}

#[test]
fn private_invite_owner_claim_from_unlisted_device_is_rejected() {
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let victim_owner = Keys::generate();
    let victim_device = Keys::generate();
    let attacker_device = Keys::generate();
    let invite_url = forged_private_invite_url(&attacker_device, victim_owner.public_key());
    let mut bob = logged_in_test_core(
        "private-accept-unlisted-owner-device",
        &bob_owner,
        &bob_device,
    );
    bob.handle_relay_event(signed_app_keys_authorization_event(
        &victim_owner,
        victim_device.public_key(),
        10,
    ));
    bob.pending_relay_publishes.clear();

    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    let excluding = signed_app_keys_authorization_event(
        &victim_owner,
        victim_device.public_key(),
        10,
    );
    ingest_invite_owner_app_keys(
        &mut bob,
        victim_owner.public_key(),
        vec![excluding],
    );

    assert!(bob.pending_outgoing_invite_acceptance.is_none());
    assert!(bob
        .state
        .toast
        .as_deref()
        .is_some_and(|toast| toast.contains("not authorized")));
    assert!(!bob
        .threads
        .contains_key(&victim_owner.public_key().to_hex()));
    assert!(bob.pending_relay_publishes.is_empty());
    assert!(active_session_device_pubkeys(&bob, victim_owner.public_key()).is_empty());
}

#[test]
fn claimed_private_invite_owner_waits_for_signed_app_keys_before_import() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let claimed_owner = Keys::generate();
    let invitee_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-waits-for-appkeys",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    let response = crafted_private_invite_response(
        &invite_url,
        &invitee_device,
        Some(invitee_device.public_key().to_hex()),
        Some(claimed_owner.public_key()),
    );

    inviter.handle_relay_event(response);

    let claimed_owner_hex = claimed_owner.public_key().to_hex();
    assert_eq!(inviter.pending_private_invite_responses.len(), 1);
    assert!(!inviter.threads.contains_key(&claimed_owner_hex));
    assert_eq!(
        inviter
            .protocol_engine
            .as_ref()
            .expect("protocol engine")
            .active_session_count_for_owner(claimed_owner.public_key()),
        0
    );
    assert!(!inviter.private_chat_invites.is_empty());
    assert!(inviter
        .compute_protocol_subscription_plan()
        .is_some_and(|plan| plan.roster_authors.contains(&claimed_owner_hex)));

    let authorization = signed_app_keys_authorization_event(
        &claimed_owner,
        invitee_device.public_key(),
        10,
    );
    ingest_invite_owner_app_keys(
        &mut inviter,
        claimed_owner.public_key(),
        vec![authorization],
    );

    assert!(inviter.pending_private_invite_responses.is_empty());
    assert!(inviter.private_chat_invites.is_empty());
    assert!(inviter.threads.contains_key(&claimed_owner_hex));
    assert_eq!(
        active_session_device_pubkeys(&inviter, claimed_owner.public_key()),
        vec![invitee_device.public_key()]
    );
}

 #[test]
fn pending_response_write_failure_stays_retryable_and_never_falls_through() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-stage-retry",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    let response = crafted_private_invite_response(
        &invite_url,
        &peer_device,
        Some(peer_device.public_key().to_hex()),
        Some(peer_owner.public_key()),
    );

    {
        let shared = inviter.app_store.shared();
        let conn = shared.lock().expect("storage connection");
        conn.execute_batch(
            "CREATE TEMP TRIGGER fail_pending_invite_response_insert
             BEFORE INSERT ON ndr_kv
             WHEN NEW.key LIKE 'pending-private-invite-responses/%'
             BEGIN SELECT RAISE(ABORT, 'injected pending write failure'); END;",
        )
        .expect("install pending write failure trigger");
    }

    inviter.handle_relay_event(response.clone());

    assert!(!inviter.has_seen_event(&response.id.to_string()));
    assert!(inviter.pending_private_invite_responses.is_empty());
    assert!(!inviter.private_chat_invites.is_empty());
    assert!(active_session_device_pubkeys(&inviter, peer_owner.public_key()).is_empty());

    {
        let shared = inviter.app_store.shared();
        let conn = shared.lock().expect("storage connection");
        conn.execute_batch("DROP TRIGGER fail_pending_invite_response_insert;")
            .expect("remove pending write failure trigger");
    }
    inviter.handle_relay_event(response.clone());

    assert!(inviter.has_seen_event(&response.id.to_string()));
    assert_eq!(inviter.pending_private_invite_responses.len(), 1);
    assert!(!inviter.private_chat_invites.is_empty());
    assert!(active_session_device_pubkeys(&inviter, peer_owner.public_key()).is_empty());
}

 #[test]
fn expired_pending_response_is_removed_without_consuming_invite() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-local-ttl",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    inviter.handle_relay_event(crafted_private_invite_response(
        &invite_url,
        &peer_device,
        Some(peer_device.public_key().to_hex()),
        Some(peer_owner.public_key()),
    ));
    for pending in inviter.pending_private_invite_responses.values_mut() {
        pending.queued_at_secs = unix_now()
            .get()
            .saturating_sub(
                super::invites::PENDING_PRIVATE_INVITE_RESPONSE_TTL_SECS,
            );
    }

    inviter.prune_pending_private_invite_responses();

    assert!(inviter.pending_private_invite_responses.is_empty());
    assert!(!inviter.private_chat_invites.is_empty());
}

#[test]
fn private_invite_owner_spoof_is_quarantined_despite_authorized_payload_device_id() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let victim_owner = Keys::generate();
    let victim_device = Keys::generate();
    let attacker_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-device-id-spoof",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    inviter.handle_relay_event(signed_app_keys_authorization_event(
        &victim_owner,
        victim_device.public_key(),
        10,
    ));
    let response = crafted_private_invite_response(
        &invite_url,
        &attacker_device,
        Some(victim_device.public_key().to_hex()),
        Some(victim_owner.public_key()),
    );

    inviter.handle_relay_event(response);

    let victim_hex = victim_owner.public_key().to_hex();
    assert_eq!(inviter.pending_private_invite_responses.len(), 1);
    assert!(!inviter.private_chat_invites.is_empty());
    assert!(!inviter.threads.contains_key(&victim_hex));
    assert!(active_session_device_pubkeys(&inviter, victim_owner.public_key()).is_empty());
    assert!(inviter.debug_log.iter().any(|entry| {
            entry.detail.contains("action=await_app_keys")
            && entry.detail.contains(&attacker_device.public_key().to_hex())
    }));
}

 #[test]
fn verified_private_invite_response_import_ignores_payload_device_id() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let authenticated_device = Keys::generate();
    let spoofed_payload_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-ignore-device-id",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    let app_keys = AppKeys::new(vec![
        DeviceEntry::new(authenticated_device.public_key(), 10),
        DeviceEntry::new(spoofed_payload_device.public_key(), 10),
    ])
    .get_event_at(peer_owner.public_key(), 10)
    .sign_with_keys(&peer_owner)
    .expect("signed peer AppKeys");
    inviter.handle_relay_event(app_keys.clone());
    let response = crafted_private_invite_response(
        &invite_url,
        &authenticated_device,
        Some(spoofed_payload_device.public_key().to_hex()),
        Some(peer_owner.public_key()),
    );

    inviter.handle_relay_event(response);
    ingest_invite_owner_app_keys(
        &mut inviter,
        peer_owner.public_key(),
        vec![app_keys],
    );

    assert_eq!(
        active_session_device_pubkeys(&inviter, peer_owner.public_key()),
        vec![authenticated_device.public_key()],
        "the cryptographically authenticated invitee identity must select the imported device"
    );
}

#[test]
fn pending_private_invite_response_survives_restart() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let mut inviter = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.clone(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    inviter.preferences.nostr_relay_urls.clear();
    inviter
        .start_primary_session(inviter_owner.clone(), inviter_device.clone(), false, false)
        .expect("start inviter");
    let invite_url = create_private_invite_for_test(&mut inviter);
    let response = crafted_private_invite_response(
        &invite_url,
        &peer_device,
        Some(peer_device.public_key().to_hex()),
        Some(peer_owner.public_key()),
    );
    inviter.handle_relay_event(response);
    assert_eq!(inviter.pending_private_invite_responses.len(), 1);
    drop(inviter);

    let mut restarted = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    restarted.preferences.nostr_relay_urls.clear();
    restarted
        .start_primary_session(inviter_owner, inviter_device, true, true)
        .expect("restart inviter");

    assert_eq!(restarted.pending_private_invite_responses.len(), 1);
    assert!(!restarted
        .threads
        .contains_key(&peer_owner.public_key().to_hex()));
    assert!(active_session_device_pubkeys(&restarted, peer_owner.public_key()).is_empty());

    let authorization = signed_app_keys_authorization_event(
        &peer_owner,
        peer_device.public_key(),
        10,
    );
    restarted.handle_relay_event(authorization);

    assert!(restarted.pending_private_invite_responses.is_empty());
    assert_eq!(
        active_session_device_pubkeys(&restarted, peer_owner.public_key()),
        vec![peer_device.public_key()]
    );
}

#[test]
fn legacy_ownerless_private_invite_response_is_self_authenticated() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-ownerless",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    let response = crafted_private_invite_response(
        &invite_url,
        &peer_device,
        Some("untrusted-display-id".to_string()),
        None,
    );

    inviter.handle_relay_event(response);

    assert!(inviter.pending_private_invite_responses.is_empty());
    assert_eq!(
        active_session_device_pubkeys(&inviter, peer_device.public_key()),
        vec![peer_device.public_key()]
    );
    assert!(inviter
        .threads
        .contains_key(&peer_device.public_key().to_hex()));
}

#[test]
fn explicit_self_owned_private_invite_response_is_self_authenticated() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-explicit-self-owner",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    let response = crafted_private_invite_response(
        &invite_url,
        &peer_device,
        Some(peer_device.public_key().to_hex()),
        Some(peer_device.public_key()),
    );

    inviter.handle_relay_event(response);

    assert!(inviter.pending_private_invite_responses.is_empty());
    assert_eq!(
        active_session_device_pubkeys(&inviter, peer_device.public_key()),
        vec![peer_device.public_key()]
    );
    assert!(inviter
        .threads
        .contains_key(&peer_device.public_key().to_hex()));
}

  #[test]
fn pending_private_invite_responses_are_bounded_per_invite() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-per-invite-bound",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    for _ in 0..6 {
        let peer_device = Keys::generate();
        inviter.handle_relay_event(crafted_private_invite_response(
            &invite_url,
            &peer_device,
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        ));
    }

    assert_eq!(
        inviter.pending_private_invite_responses.len(),
        super::invites::PENDING_PRIVATE_INVITE_RESPONSE_PER_INVITE_LIMIT
    );
    assert!(!inviter.private_chat_invites.is_empty());
}
