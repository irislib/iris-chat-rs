use super::*;

include!("protocol_engine/types.rs");
include!("protocol_engine/engine_core.rs");
include!("protocol_engine/engine_fact_ingest.rs");
include!("protocol_engine/engine_sends.rs");
include!("protocol_engine/roster_helpers.rs");
include!("protocol_engine/engine_incoming_retry.rs");
include!("protocol_engine/engine_resolution.rs");
include!("protocol_engine/engine_sender_key_repair.rs");
include!("protocol_engine/engine_persistence.rs");
include!("protocol_engine/free_functions.rs");

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
        let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), device)
            .expect("protocol engine")
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

    fn group_snapshot_for_test(
        group_id: &str,
        name: &str,
        revision: u64,
        admin: &Keys,
        members: &[PublicKey],
    ) -> GroupSnapshot {
        GroupSnapshot {
            group_id: group_id.to_string(),
            protocol: GroupProtocol::sender_key_v1(),
            name: name.to_string(),
            picture: None,
            about: None,
            created_by: ndr_owner(admin.public_key()),
            members: members.iter().copied().map(ndr_owner).collect(),
            admins: vec![ndr_owner(admin.public_key())],
            revision,
            created_at: NdrUnixSeconds(10),
            updated_at: NdrUnixSeconds(10 + revision),
        }
    }

    fn group_roster_fact_event_for_test(admin: &Keys, snapshot: &GroupSnapshot) -> Event {
        group_roster_unsigned_event(admin.public_key(), snapshot)
            .expect("unsigned group roster fact")
            .sign_with_keys(admin)
            .expect("signed group roster fact")
    }
}
