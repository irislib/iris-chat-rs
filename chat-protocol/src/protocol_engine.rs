use super::*;

include!("protocol_engine/types.rs");
include!("protocol_engine/engine_core.rs");
include!("protocol_engine/engine_state_helpers.rs");
include!("protocol_engine/engine_fact_ingest.rs");
include!("protocol_engine/engine_sends.rs");
include!("protocol_engine/engine_invite_owner.rs");
include!("protocol_engine/roster_helpers.rs");
include!("protocol_engine/engine_incoming_retry.rs");
include!("protocol_engine/engine_resolution.rs");
include!("protocol_engine/engine_sender_key_repair.rs");
include!("protocol_engine/engine_persistence.rs");
include!("protocol_engine/free_functions.rs");

#[cfg(test)]
mod tests {
    use super::*;

    include!("protocol_engine/invite_owner_tests.rs");

    fn read_protocol_engine_source(path: &str) -> String {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
            .unwrap_or_else(|error| panic!("read {path}: {error}"))
    }

    #[test]
    fn sender_owner_resolution_keeps_claimed_device_pending_until_owner_verified() {
        let protocol_source =
            read_protocol_engine_source("src/protocol_engine/engine_resolution.rs");
        let start = protocol_source
            .find("fn resolve_message_sender_owner")
            .expect("sender resolver");
        let body = &protocol_source[start
            ..protocol_source[start..]
                .find("\n    fn ensure_local_roster")
                .map(|offset| start + offset)
                .unwrap_or(protocol_source.len())];
        assert!(
            body.contains("PendingOwnerClaim"),
            "claimed owners must be represented as pending, not collapsed into a device owner"
        );
        assert!(
            !body.contains("NdrOwnerPubkey::from_bytes(envelope.sender.to_bytes())"),
            "message envelope sender is a ratchet sender key and must not become the canonical owner"
        );
    }

    #[test]
    fn pending_inbound_owner_targets_use_cached_metadata_not_event_reparse() {
        let protocol_source =
            read_protocol_engine_source("src/protocol_engine/engine_resolution.rs");
        let start = protocol_source
            .find("fn pending_inbound_owner_claim_targets")
            .expect("pending inbound target collector");
        let body = &protocol_source[start
            ..protocol_source[start..]
                .find("\n    fn pending_group_pairwise_owner_claim_targets")
                .map(|offset| start + offset)
                .unwrap_or(protocol_source.len())];
        assert!(
            body.contains("claimed_owner_pubkey_hex"),
            "pending inbound owner target collection must use cached owner metadata"
        );
        assert!(
            !body.contains("parse_message_event"),
            "pending inbound owner target collection runs on the relay hot path and must not verify every pending event"
        );
    }

    #[test]
    fn group_sender_key_ignored_results_are_consumed_without_retry_queue() {
        let incoming_source =
            read_protocol_engine_source("src/protocol_engine/engine_incoming_retry.rs");
        let repair_source =
            read_protocol_engine_source("src/protocol_engine/engine_sender_key_repair.rs");
        let process_start = incoming_source
            .find("fn process_group_outer_event")
            .expect("process group outer function");
        let process_body = &incoming_source[process_start
            ..incoming_source[process_start..]
                .find("fn process_group_pairwise_payload")
                .map(|offset| process_start + offset)
                .unwrap_or(incoming_source.len())];
        assert!(
            process_body.contains("if result.pending"),
            "group outer handling must queue sender-key messages only for explicit pending results"
        );
        assert!(
            !process_body.contains("if result.events.is_empty()"),
            "ignored sender-key results have no events but must not be queued for retry"
        );

        let handle_start = repair_source
            .find("fn handle_group_sender_key_message")
            .expect("handle sender key function");
        let handle_body = &repair_source[handle_start
            ..repair_source[handle_start..]
                .find("fn sender_key_repair_request_effects")
                .map(|offset| handle_start + offset)
                .unwrap_or(repair_source.len())];
        assert!(
            handle_body.contains("GroupSenderKeyHandleResult::Ignored")
                && handle_body.contains("consumed: true"),
            "ignored parsed sender-key events should be consumed so relay replays do not loop"
        );
    }

    fn test_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
        let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), device)
            .expect("protocol engine")
    }

    fn claimed_owner_invite(owner: &Keys, device: &Keys) -> Invite {
        let mut invite = Invite::create_new(
            device.public_key(),
            Some(device.public_key().to_hex()),
            Some(1),
        )
        .expect("invite");
        invite.owner_public_key = Some(owner.public_key());
        invite.inviter_owner_pubkey = Some(ndr_owner(owner.public_key()));
        invite.purpose = Some("private".to_string());
        invite
    }

    fn signed_app_keys(owner: &Keys, devices: &[PublicKey], created_at: u64) -> Event {
        AppKeys::new(
            devices
                .iter()
                .copied()
                .map(|device| DeviceEntry::new(device, created_at))
                .collect(),
        )
        .get_event_at(owner.public_key(), created_at)
        .sign_with_keys(owner)
        .expect("signed AppKeys")
    }

    fn strip_app_keys_provenance_from_persisted_state(storage: &dyn StorageAdapter) {
        let raw = storage
            .get(PROTOCOL_ENGINE_STATE_KEY)
            .expect("read protocol state")
            .expect("persisted protocol state");
        let mut state =
            serde_json::from_str::<serde_json::Value>(&raw).expect("protocol state json");
        state
            .as_object_mut()
            .expect("protocol state object")
            .remove("verified_app_keys_owners");
        state
            .as_object_mut()
            .expect("protocol state object")
            .remove("app_keys_provenance_version");
        state
            .as_object_mut()
            .expect("protocol state object")
            .remove("invite_owner_app_keys_evidence");
        storage
            .put(
                PROTOCOL_ENGINE_STATE_KEY,
                serde_json::to_string(&state).expect("serialize legacy protocol state"),
            )
            .expect("write legacy protocol state");
    }

    #[test]
    fn conflicting_invite_owner_representations_are_rejected() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let claimed_owner = Keys::generate();
        let conflicting_owner = Keys::generate();
        let inviter_device = Keys::generate();
        let mut invite = claimed_owner_invite(&claimed_owner, &inviter_device);
        invite.owner_public_key = Some(conflicting_owner.public_key());
        let mut engine = test_engine(&local_owner, &local_device);

        let embedded_error = engine
            .accept_invite(&invite, None)
            .expect_err("conflicting embedded owners must fail");
        assert!(embedded_error
            .to_string()
            .contains("invite owner fields disagree"));

        invite.owner_public_key = Some(claimed_owner.public_key());
        let hint_error = engine
            .accept_invite(&invite, Some(conflicting_owner.public_key()))
            .expect_err("conflicting owner hint must fail");
        assert!(hint_error
            .to_string()
            .contains("invite owner hint disagrees"));
        assert_eq!(
            engine.active_session_count_for_owner(claimed_owner.public_key()),
            0
        );
        assert_eq!(
            engine.active_session_count_for_owner(conflicting_owner.public_key()),
            0
        );
    }

    #[test]
    fn invite_owner_claim_waits_for_signed_roster_membership() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let claimed_owner = Keys::generate();
        let inviter_device = Keys::generate();
        let unrelated_device = Keys::generate();
        let invite = claimed_owner_invite(&claimed_owner, &inviter_device);
        let mut engine = test_engine(&local_owner, &local_device);

        let missing = engine
            .accept_invite(&invite, Some(claimed_owner.public_key()))
            .expect("missing roster outcome");
        assert!(matches!(
            missing,
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::MissingOwnerRoster { .. }
            )
        ));

        let unrelated_roster =
            signed_app_keys(&claimed_owner, &[unrelated_device.public_key()], 10);
        engine
            .ingest_app_keys_event(&unrelated_roster)
            .expect("ordinary unrelated roster");
        let excluded = engine
            .accept_invite(&invite, Some(claimed_owner.public_key()))
            .expect("ordinary signed AppKeys excludes invite device");
        assert!(matches!(
            excluded,
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::UnauthorizedDevice { .. }
            )
        ));
        assert_eq!(
            engine.active_session_count_for_owner(claimed_owner.public_key()),
            0
        );
    }

    #[test]
    fn ordinary_app_keys_heads_are_newest_wins_and_conflicts_fail_closed() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let claimed_owner = Keys::generate();
        let claimed_device = Keys::generate();
        let other_device = Keys::generate();
        let invite = claimed_owner_invite(&claimed_owner, &claimed_device);
        let inclusion = signed_app_keys(&claimed_owner, &[claimed_device.public_key()], 10);

        let newer_exclusion = signed_app_keys(&claimed_owner, &[other_device.public_key()], 20);
        let mut vetoed = test_engine(&local_owner, &local_device);
        vetoed.ingest_app_keys_event(&inclusion).unwrap();
        vetoed.ingest_app_keys_event(&newer_exclusion).unwrap();
        assert!(matches!(
            vetoed
                .accept_invite(&invite, Some(claimed_owner.public_key()))
                .expect("newer incomplete exclusion"),
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::UnauthorizedDevice { .. }
            )
        ));

        let equal_exclusion = signed_app_keys(&claimed_owner, &[other_device.public_key()], 10);
        let mut ambiguous = test_engine(&local_owner, &local_device);
        ambiguous.ingest_app_keys_event(&inclusion).unwrap();
        ambiguous.ingest_app_keys_event(&equal_exclusion).unwrap();
        assert!(matches!(
            ambiguous
                .accept_invite(&invite, Some(claimed_owner.public_key()))
                .expect("equal timestamp conflict"),
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::MissingOwnerRoster { .. }
            )
        ));
    }

    #[test]
    fn private_invite_session_import_is_idempotent_across_restart() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let storage = Arc::new(InMemoryStorage::new());
        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut engine = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("protocol engine");
        let roster = signed_app_keys(&peer_owner, &[peer_device.public_key()], 10);
        engine.ingest_app_keys_event(&roster).expect("owner roster");

        let session_state = || {
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(10), &mut rng);
            nostr_double_ratchet::Session::new_responder(
                &mut ctx,
                ndr_device(peer_device.public_key()),
                Keys::generate().secret_key().to_secret_bytes(),
                [7; 32],
            )
            .expect("session")
            .state
        };
        assert!(matches!(
            engine
                .import_private_invite_session_once(
                    "response-id",
                    peer_owner.public_key(),
                    peer_device.public_key(),
                    session_state(),
                    UnixSeconds(10),
                )
                .expect("first import"),
            ProtocolInviteSessionImportOutcome::Imported(_)
        ));
        assert!(matches!(
            engine
                .import_private_invite_session_once(
                    "response-id",
                    peer_owner.public_key(),
                    peer_device.public_key(),
                    session_state(),
                    UnixSeconds(11),
                )
                .expect("duplicate import"),
            ProtocolInviteSessionImportOutcome::AlreadyImported
        ));
        assert_eq!(
            engine.active_session_count_for_owner(peer_owner.public_key()),
            1
        );
        drop(engine);

        let adapter: Arc<dyn StorageAdapter> = storage;
        let mut restarted = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("restarted engine");
        assert!(matches!(
            restarted
                .import_private_invite_session_once(
                    "response-id",
                    peer_owner.public_key(),
                    peer_device.public_key(),
                    session_state(),
                    UnixSeconds(12),
                )
                .expect("duplicate import after restart"),
            ProtocolInviteSessionImportOutcome::AlreadyImported
        ));
        assert_eq!(
            restarted.active_session_count_for_owner(peer_owner.public_key()),
            1
        );
    }

    #[test]
    fn legacy_synthetic_owner_roster_is_quarantined_until_real_app_keys_arrive() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let alice = Keys::generate();
        let attacker_device = Keys::generate();
        let attacker_message_key = Keys::generate();
        let real_alice_device = Keys::generate();
        let forged_invite = claimed_owner_invite(&alice, &attacker_device);
        let storage = Arc::new(InMemoryStorage::new());
        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut legacy = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("legacy engine");
        let alice_owner = ndr_owner(alice.public_key());
        let attacker = ndr_device(attacker_device.public_key());

        // Model the pre-fix state: accepting an owner hint synthesized a roster
        // with the attacker's key and cached it as authorized under Alice.
        legacy
            .session_manager
            .observe_device_invite(alice_owner, forged_invite.clone())
            .expect("legacy forged invite");
        legacy.session_manager.observe_peer_roster(
            alice_owner,
            DeviceRoster::new(
                NdrUnixSeconds(1_000),
                vec![AuthorizedDevice::new(attacker, NdrUnixSeconds(1_000))],
            ),
        );
        let attacker_sender = ndr_device(attacker_message_key.public_key());
        let responder_keys = Keys::generate();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(1_000), &mut rng);
        let legacy_session = nostr_double_ratchet::Session::new_responder(
            &mut ctx,
            attacker_sender,
            responder_keys.secret_key().to_secret_bytes(),
            [7; 32],
        )
        .expect("legacy attacker session");
        legacy.session_manager.import_session_state(
            alice_owner,
            attacker,
            legacy_session.state,
            NdrUnixSeconds(1_000),
        );
        legacy.persist_now().expect("persist poisoned legacy state");
        strip_app_keys_provenance_from_persisted_state(storage.as_ref());
        drop(legacy);

        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut migrated = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("migrated engine");
        let migrated_alice = migrated
            .session_manager_snapshot_for_test()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == alice_owner)
            .expect("quarantined Alice record");
        assert!(migrated_alice.roster.is_none());
        assert!(migrated_alice
            .devices
            .iter()
            .any(|device| device.device_pubkey == attacker && !device.authorized));
        assert_eq!(
            migrated.resolve_message_sender_owner_for_sender(attacker_sender),
            ProtocolSenderOwnerResolution::PendingOwnerClaim {
                storage_owner: provisional_owner_from_sender_pubkey(attacker),
                claimed_owner: alice_owner,
                sender_device: attacker,
            }
        );
        assert_eq!(
            migrated.active_session_count_for_owner(alice.public_key()),
            0,
            "quarantined sessions must not count as active owner sessions"
        );
        assert!(migrated
            .known_message_author_pubkeys()
            .contains(&attacker_message_key.public_key()));
        assert!(migrated
            .known_device_identity_pubkeys_for_owner(alice.public_key())
            .is_empty());
        assert!(!migrated
            .known_verified_peer_owner_pubkeys()
            .contains(&alice.public_key()));
        assert!(migrated
            .verified_message_session_snapshots_for_owner(alice.public_key())
            .is_empty());
        assert!(!migrated
            .has_device_roster_entry_for_owner(alice.public_key(), attacker_device.public_key(),));
        assert!(matches!(
            migrated
                .accept_invite(&forged_invite, Some(alice.public_key()))
                .expect("unproven owner outcome"),
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::MissingOwnerRoster { .. }
            )
        ));

        // The real signed snapshot is older than the poisoned synthetic roster.
        // Quarantine must prevent that synthetic timestamp from outranking it.
        let real_alice_roster = signed_app_keys(&alice, &[real_alice_device.public_key()], 10);
        migrated
            .ingest_app_keys_event(&real_alice_roster)
            .expect("trusted Alice AppKeys");
        let alice_roster = migrated
            .session_manager_snapshot_for_test()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == alice_owner)
            .and_then(|user| user.roster)
            .expect("verified Alice roster");
        assert_eq!(alice_roster.created_at, NdrUnixSeconds(10));
        assert!(alice_roster
            .get_device(&ndr_device(real_alice_device.public_key()))
            .is_some());
        assert!(alice_roster.get_device(&attacker).is_none());
        assert!(matches!(
            migrated
                .accept_invite(&forged_invite, Some(alice.public_key()))
                .expect("signed roster excludes forged invite device"),
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::UnauthorizedDevice { .. }
            )
        ));
        assert_eq!(
            migrated.owner_resolution_for_sender_record(ProtocolSenderDeviceRecord {
                storage_owner: alice_owner,
                device_pubkey: ndr_device(real_alice_device.public_key()),
                claimed_owner_pubkey: None,
            }),
            ProtocolSenderOwnerResolution::Verified { owner: alice_owner }
        );
        assert!(matches!(
            migrated.resolve_message_sender_owner_for_sender(attacker_sender),
            ProtocolSenderOwnerResolution::PendingOwnerClaim { .. }
        ));

        // Provenance itself must survive restart; otherwise a real roster would
        // be quarantined again on every launch.
        drop(migrated);
        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let restarted = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("restarted engine");
        assert_eq!(
            restarted.owner_resolution_for_sender_record(ProtocolSenderDeviceRecord {
                storage_owner: alice_owner,
                device_pubkey: ndr_device(real_alice_device.public_key()),
                claimed_owner_pubkey: None,
            }),
            ProtocolSenderOwnerResolution::Verified { owner: alice_owner }
        );
        assert!(matches!(
            restarted.resolve_message_sender_owner_for_sender(attacker_sender),
            ProtocolSenderOwnerResolution::PendingOwnerClaim { .. }
        ));
    }

    #[test]
    fn legacy_self_owned_singleton_roster_remains_usable_without_provenance() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let peer_device = Keys::generate();
        let peer = ndr_device(peer_device.public_key());
        let peer_owner = ndr_owner(peer_device.public_key());
        let invite = Invite::create_new(
            peer_device.public_key(),
            Some(peer_device.public_key().to_hex()),
            Some(1),
        )
        .expect("self-owned invite");
        let storage = Arc::new(InMemoryStorage::new());
        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut legacy = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("legacy engine");
        legacy.session_manager.observe_peer_roster(
            peer_owner,
            DeviceRoster::new(
                NdrUnixSeconds(1),
                vec![AuthorizedDevice::new(peer, NdrUnixSeconds(1))],
            ),
        );
        legacy.persist_now().expect("persist legacy singleton");
        strip_app_keys_provenance_from_persisted_state(storage.as_ref());
        drop(legacy);

        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut migrated = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("migrated engine");
        let roster = migrated
            .session_manager_snapshot_for_test()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == peer_owner)
            .and_then(|user| user.roster)
            .expect("self-owned singleton roster");
        assert_eq!(
            roster.devices(),
            &[AuthorizedDevice::new(peer, NdrUnixSeconds(1))]
        );
        assert!(matches!(
            migrated
                .accept_invite(&invite, None)
                .expect("self-owned invite acceptance"),
            ProtocolAcceptInviteOutcome::Accepted(_)
        ));
    }

    #[test]
    fn legacy_local_sibling_invite_acceptance_requires_signed_app_keys() {
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let forged_sibling = Keys::generate();
        let forged_invite = claimed_owner_invite(&local_owner, &forged_sibling);
        let storage = Arc::new(InMemoryStorage::new());
        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut legacy = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("legacy engine");
        legacy
            .session_manager
            .observe_device_invite(ndr_owner(local_owner.public_key()), forged_invite.clone())
            .expect("legacy local-owner invite");
        legacy
            .session_manager
            .replace_local_roster(DeviceRoster::new(
                NdrUnixSeconds(1_000),
                vec![
                    AuthorizedDevice::new(
                        ndr_device(local_device.public_key()),
                        NdrUnixSeconds(1_000),
                    ),
                    AuthorizedDevice::new(
                        ndr_device(forged_sibling.public_key()),
                        NdrUnixSeconds(1_000),
                    ),
                ],
            ));
        legacy.persist_now().expect("persist poisoned local roster");
        strip_app_keys_provenance_from_persisted_state(storage.as_ref());
        drop(legacy);

        let adapter: Arc<dyn StorageAdapter> = storage.clone();
        let mut migrated = ProtocolEngine::load_or_create_for_local_device(
            adapter,
            local_owner.public_key(),
            &local_device,
        )
        .expect("migrated engine");
        assert!(migrated.is_known_local_owner_device(local_device.public_key()));
        assert!(!migrated.is_known_local_owner_device(forged_sibling.public_key()));

        migrated
            .ingest_app_keys_snapshot(
                local_owner.public_key(),
                AppKeys::new(vec![
                    DeviceEntry::new(local_device.public_key(), 10),
                    DeviceEntry::new(forged_sibling.public_key(), 10),
                ]),
                10,
            )
            .expect("reconstructed local AppKeys");
        assert!(migrated.is_known_local_owner_device(forged_sibling.public_key()));
        assert!(matches!(
            migrated
                .accept_invite(&forged_invite, Some(local_owner.public_key()))
                .expect("reconstructed roster must not authorize invite acceptance"),
            ProtocolAcceptInviteOutcome::Blocked(
                ProtocolAcceptInviteBlock::MissingOwnerRoster { .. }
            )
        ));
        let signed = signed_app_keys(
            &local_owner,
            &[local_device.public_key(), forged_sibling.public_key()],
            10,
        );
        migrated
            .ingest_app_keys_event(&signed)
            .expect("signed local AppKeys");
        assert!(migrated.is_known_local_owner_device(forged_sibling.public_key()));
    }

    #[test]
    fn group_roster_fact_events_update_group_snapshot() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let admin = Keys::generate();
        let member = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        let snapshot = group_snapshot_for_test(
            "group-roster-fact",
            "Fact Group",
            3,
            &admin,
            &[admin.public_key(), member.public_key(), owner.public_key()],
        );
        let event = group_roster_fact_event_for_test(&admin, &snapshot);

        let result = engine
            .ingest_group_roster_fact_event(&event)
            .expect("group roster fact")
            .expect("valid fact should be consumed");

        assert_eq!(
            result.snapshot.as_ref().map(|group| group.revision),
            Some(3)
        );
        let installed = engine
            .group_manager
            .group("group-roster-fact")
            .expect("installed group");
        assert_eq!(installed.name, "Fact Group");
        assert_eq!(installed.members.len(), 3);
    }

    #[test]
    fn group_roster_fact_ingest_syncs_installed_group_to_local_siblings() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sibling = Keys::generate();
        let admin = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        engine
            .session_manager
            .replace_local_roster(DeviceRoster::new(
                NdrUnixSeconds(1),
                vec![
                    AuthorizedDevice::new(ndr_device(device.public_key()), NdrUnixSeconds(1)),
                    AuthorizedDevice::new(ndr_device(sibling.public_key()), NdrUnixSeconds(1)),
                ],
            ));
        let snapshot = group_snapshot_for_test(
            "group-roster-fact-local-sibling",
            "Fact Group",
            1,
            &admin,
            &[admin.public_key(), owner.public_key()],
        );
        let event = group_roster_fact_event_for_test(&admin, &snapshot);

        let result = engine
            .ingest_group_roster_fact_event(&event)
            .expect("group roster fact")
            .expect("valid fact should be consumed");

        assert_eq!(
            result
                .snapshot
                .as_ref()
                .map(|group| group.group_id.as_str()),
            Some("group-roster-fact-local-sibling")
        );
        assert!(
            engine.has_pending_retry_work(),
            "installed public roster facts should leave local-sibling sync as pending state when no local-sibling session is available"
        );
    }

    #[test]
    fn group_roster_fact_history_keeps_newest_snapshot() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let admin = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        let old = group_snapshot_for_test(
            "group-roster-fact",
            "Old Group",
            1,
            &admin,
            &[admin.public_key(), owner.public_key()],
        );
        let new = GroupSnapshot {
            name: "New Group".to_string(),
            revision: 2,
            updated_at: NdrUnixSeconds(20),
            ..old.clone()
        };

        engine
            .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&admin, &new))
            .expect("new fact");
        let stale_result = engine
            .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&admin, &old))
            .expect("old fact")
            .expect("stale valid fact should be consumed");

        assert!(stale_result.snapshot.is_none());
        let installed = engine
            .group_manager
            .group("group-roster-fact")
            .expect("installed group");
        assert_eq!(installed.name, "New Group");
        assert_eq!(installed.revision, 2);
    }

    #[test]
    fn group_roster_fact_rejects_update_not_signed_by_existing_admin() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let admin = Keys::generate();
        let attacker = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        let original = group_snapshot_for_test(
            "group-roster-admin-check",
            "Original Group",
            1,
            &admin,
            &[admin.public_key(), owner.public_key()],
        );
        engine
            .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&admin, &original))
            .expect("original fact")
            .expect("original fact consumed");

        let malicious = group_snapshot_for_test(
            "group-roster-admin-check",
            "Pwned Group",
            2,
            &attacker,
            &[attacker.public_key(), owner.public_key()],
        );
        let result = engine
            .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(
                &attacker, &malicious,
            ))
            .expect("malicious fact is ignored without failing sync");

        assert!(result.is_none());
        let installed = engine
            .group_manager
            .group("group-roster-admin-check")
            .expect("installed group");
        assert_eq!(installed.name, "Original Group");
        assert_eq!(installed.revision, 1);
    }

    #[test]
    fn deferred_sender_key_repair_response_is_throttled() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let requester = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        let created = engine
            .create_group(
                "Deferred repair response".to_string(),
                vec![requester.public_key()],
                UnixSeconds(100),
            )
            .expect("create group");
        let group = created.snapshot.expect("created group snapshot");
        engine.pending_group_fanouts.clear();

        let request = SenderKeyRepairRequest {
            group_id: group.group_id,
            sender_event_pubkey: ndr_device(Keys::generate().public_key()),
            key_id: None,
            message_number: None,
            required_revision: Some(group.revision),
            created_at: NdrUnixSeconds(101),
        };
        let requester_owner = ndr_owner(requester.public_key());

        let first = engine
            .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(102))
            .expect("prepare first repair response");
        assert!(
            first.is_empty(),
            "response must wait for a pairwise session"
        );
        assert_eq!(engine.pending_group_fanouts.len(), 1);
        assert_eq!(engine.answered_group_sender_key_repairs.len(), 1);

        let pending_fanouts = engine.pending_group_fanouts.clone();
        let answered_repairs = engine.answered_group_sender_key_repairs.clone();
        let sessions = engine.session_manager.snapshot();
        let groups = engine.group_manager.snapshot();

        let duplicate = engine
            .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(103))
            .expect("process duplicate repair request");
        assert!(duplicate.is_empty());
        assert_eq!(engine.pending_group_fanouts, pending_fanouts);
        assert_eq!(engine.answered_group_sender_key_repairs, answered_repairs);
        assert_eq!(engine.session_manager.snapshot(), sessions);
        assert_eq!(engine.group_manager.snapshot(), groups);
    }

    #[test]
    fn reload_compacts_superseded_local_group_sync_fanouts() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
        let mut engine = ProtocolEngine::load_or_create_for_local_device(
            Arc::clone(&storage),
            owner.public_key(),
            &device,
        )
        .expect("protocol engine");
        engine.pending_group_fanouts = vec![
            pending_local_group_fanout("group-a", None, b"old", 10),
            pending_local_group_fanout("group-b", None, b"other", 11),
            pending_local_group_fanout("group-a", None, b"new", 12),
            pending_local_group_fanout("group-a", Some("message-1"), b"message", 13),
        ];
        engine.persist().expect("persist legacy fanout queue");
        drop(engine);

        let reloaded =
            ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
                .expect("reload protocol engine");

        assert_eq!(reloaded.pending_group_fanouts.len(), 3);
        assert!(reloaded.pending_group_fanouts.iter().any(|pending| {
            pending.group_id == "group-a"
                && pending.inner_event_id.is_none()
                && matches!(
                    &pending.fanout,
                    GroupPendingFanout::LocalSiblings { payload } if payload == b"new"
                )
        }));
        assert!(reloaded
            .pending_group_fanouts
            .iter()
            .any(|pending| pending.inner_event_id.as_deref() == Some("message-1")));
    }

    #[test]
    fn local_group_sync_enqueue_replaces_obsolete_pending_payload() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        let mut prepared = GroupPreparedPublish::empty();
        prepared.pending_fanouts = vec![GroupPendingFanout::LocalSiblings {
            payload: b"old".to_vec(),
        }];
        engine.queue_group_pending_fanouts("group-a", &prepared, None);
        prepared.pending_fanouts = vec![GroupPendingFanout::LocalSiblings {
            payload: b"new".to_vec(),
        }];
        engine.queue_group_pending_fanouts("group-a", &prepared, None);

        assert_eq!(engine.pending_group_fanouts.len(), 1);
        assert!(matches!(
            &engine.pending_group_fanouts[0].fanout,
            GroupPendingFanout::LocalSiblings { payload } if payload == b"new"
        ));
    }

    #[test]
    fn group_fanout_retry_processes_a_bounded_due_batch() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        for index in 0..70 {
            engine
                .pending_group_fanouts
                .push(pending_local_group_fanout(
                    "group-a",
                    Some(&format!("message-{index}")),
                    &[index as u8],
                    1,
                ));
        }

        engine
            .retry_pending_group_fanouts(NdrUnixSeconds(10))
            .expect("retry pending fanouts");

        let due_remaining = engine
            .pending_group_fanouts
            .iter()
            .filter(|pending| pending.next_retry_at_secs <= 10)
            .count();
        assert_eq!(due_remaining, 6);
    }

    include!("protocol_engine/test_helpers.rs");
}
