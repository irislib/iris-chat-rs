#[test]
fn verified_invite_acceptance_preserves_full_signed_roster() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let claimed_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let sibling_device = Keys::generate();
    let invite = claimed_owner_invite(&claimed_owner, &inviter_device);
    let mut engine = test_engine(&local_owner, &local_device);
    let roster = signed_app_keys(
        &claimed_owner,
        &[inviter_device.public_key(), sibling_device.public_key()],
        10,
    );
    engine
        .ingest_app_keys_event(&roster)
        .expect("verified roster");

    let accepted = engine
        .accept_invite(&invite, Some(claimed_owner.public_key()))
        .expect("accept outcome");
    assert!(matches!(accepted, ProtocolAcceptInviteOutcome::Accepted(_)));

    let roster = engine
        .session_manager_snapshot_for_test()
        .users
        .into_iter()
        .find(|user| user.owner_pubkey == ndr_owner(claimed_owner.public_key()))
        .and_then(|user| user.roster)
        .expect("preserved roster");
    assert!(roster
        .get_device(&ndr_device(inviter_device.public_key()))
        .is_some());
    assert!(roster
        .get_device(&ndr_device(sibling_device.public_key()))
        .is_some());

    let second_invite = claimed_owner_invite(&claimed_owner, &inviter_device);
    let second = engine
        .accept_invite(&second_invite, Some(claimed_owner.public_key()))
        .expect("second invite outcome");
    assert!(matches!(second, ProtocolAcceptInviteOutcome::Accepted(_)));
}

#[test]
fn signed_invite_owner_evidence_survives_restart() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let claimed_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let invite = claimed_owner_invite(&claimed_owner, &inviter_device);
    let storage = Arc::new(InMemoryStorage::new());
    let adapter: Arc<dyn StorageAdapter> = storage.clone();
    let mut engine = ProtocolEngine::load_or_create_for_local_device(
        adapter,
        local_owner.public_key(),
        &local_device,
    )
    .expect("protocol engine");
    let roster = signed_app_keys(&claimed_owner, &[inviter_device.public_key()], 10);
    engine
        .ingest_app_keys_event(&roster)
        .expect("signed owner roster");
    drop(engine);

    let adapter: Arc<dyn StorageAdapter> = storage;
    assert!(ProtocolEngine::persisted_invite_owner_device_is_authorized(
        adapter.clone(),
        local_owner.public_key(),
        &local_device,
        claimed_owner.public_key(),
        inviter_device.public_key(),
    )
    .expect("persisted exact evidence"));
    let mut restarted = ProtocolEngine::load_or_create_for_local_device(
        adapter,
        local_owner.public_key(),
        &local_device,
    )
    .expect("restarted engine");
    let outcome = restarted
        .accept_invite(&invite, Some(claimed_owner.public_key()))
        .expect("restart invite outcome");
    assert!(matches!(outcome, ProtocolAcceptInviteOutcome::Accepted(_)));
}
