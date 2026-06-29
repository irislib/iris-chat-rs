use super::*;

include!("protocol_engine/types.rs");
include!("protocol_engine/engine_core.rs");
include!("protocol_engine/engine_sends.rs");
include!("protocol_engine/roster_helpers.rs");
include!("protocol_engine/engine_incoming_retry.rs");
include!("protocol_engine/engine_resolution.rs");
include!("protocol_engine/engine_sender_key_repair.rs");
include!("protocol_engine/engine_queue_filters.rs");
include!("protocol_engine/free_functions.rs");

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
        let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), device)
            .expect("protocol engine")
    }

    fn pending_outbound(
        message_id: &str,
        chat_id: &str,
        recipient_owner_hex: String,
        send_remote: bool,
        local_sibling_payload: Option<Vec<u8>>,
        probe_local_sibling_roster: bool,
        reason: ProtocolPendingReason,
    ) -> ProtocolPendingOutbound {
        ProtocolPendingOutbound {
            message_id: message_id.to_string(),
            chat_id: chat_id.to_string(),
            recipient_owner_hex,
            send_remote,
            remote_payload: b"remote".to_vec(),
            local_sibling_payload,
            inner_event_id: Some(message_id.to_string()),
            delivered_remote_device_hexes: Vec::new(),
            delivered_local_device_hexes: Vec::new(),
            probe_local_sibling_roster,
            created_at_secs: 1,
            next_retry_at_secs: 1,
            reason,
        }
    }

    #[test]
    fn local_sibling_roster_probe_does_not_block_delivery() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        engine.pending_outbound.push(pending_outbound(
            "message-id",
            "chat-id",
            owner.public_key().to_hex(),
            false,
            Some(b"local".to_vec()),
            true,
            ProtocolPendingReason::PublishRetry,
        ));

        assert!(!engine.has_delivery_blocking_message_work("message-id"));
    }

    #[test]
    fn known_local_sibling_target_blocks_delivery() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sibling = Keys::generate();
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
        engine.pending_outbound.push(pending_outbound(
            "message-id",
            "chat-id",
            owner.public_key().to_hex(),
            false,
            Some(b"local".to_vec()),
            false,
            ProtocolPendingReason::PublishRetry,
        ));

        assert!(engine.has_delivery_blocking_message_work("message-id"));
    }

    #[test]
    fn missing_remote_roster_blocks_delivery() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let mut engine = test_engine(&owner, &device);
        engine.pending_outbound.push(pending_outbound(
            "message-id",
            &peer.public_key().to_hex(),
            peer.public_key().to_hex(),
            true,
            None,
            false,
            ProtocolPendingReason::MissingRoster,
        ));

        assert!(engine.has_delivery_blocking_message_work("message-id"));
    }

    #[test]
    fn protocol_discovery_effects_fetch_device_roster_facts_and_invites_for_owner() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let engine = test_engine(&owner, &device);

        let effects = engine.protocol_discovery_effects_for_owners(
            [peer.public_key()],
            UnixSeconds(1_777_159_500),
            "test_discovery",
        );

        let filters = effects
            .into_iter()
            .flat_map(|effect| match effect {
                ProtocolEffect::FetchProtocolState { filters, .. } => filters,
                ProtocolEffect::Publish(_) => Vec::new(),
            })
            .collect::<Vec<_>>();

        assert_eq!(filters.len(), 2);
        assert!(has_filter_with_kind_author(
            &filters,
            NOSTR_IDENTITY_ROSTER_OP_KIND,
            peer.public_key()
        ));
        assert!(has_filter_with_kind_author(
            &filters,
            INVITE_EVENT_KIND,
            peer.public_key()
        ));
    }

    #[test]
    fn nostr_identity_roster_op_events_update_device_roster() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let mut engine = test_engine(&owner, &device);

        let bootstrap = nostr_identity_roster_event(
            &peer_owner,
            vec![
                tag_values(["op", "add_key"]),
                tag_values(["key_pubkey", peer_owner.public_key().to_hex().as_str()]),
                tag_values(["key_purpose", "app"]),
                tag_values(["key_capability", "admin"]),
                tag_values(["key_capability", "write"]),
                tag_values(["key_added_at", "10"]),
            ],
            10,
        );
        let add_device = nostr_identity_roster_event(
            &peer_owner,
            vec![
                tag_values(["op", "add_key"]),
                tag_values(["key_pubkey", peer_device.public_key().to_hex().as_str()]),
                tag_values(["key_purpose", "app"]),
                tag_values(["key_capability", "write"]),
                tag_values(["key_added_at", "11"]),
            ],
            11,
        );

        engine
            .ingest_nostr_identity_roster_op_event(&bootstrap)
            .expect("bootstrap fact");
        engine
            .ingest_nostr_identity_roster_op_event(&add_device)
            .expect("add device fact");

        let devices = engine.known_device_identity_pubkeys_for_owner(peer_owner.public_key());
        assert_eq!(devices.len(), 1);
        assert!(devices.contains(&peer_device.public_key()));
    }

    #[test]
    fn nostr_identity_roster_op_history_applies_tombstones() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let mut engine = test_engine(&owner, &device);

        for event in [
            nostr_identity_roster_event(
                &peer_owner,
                vec![
                    tag_values(["op", "add_key"]),
                    tag_values(["key_pubkey", peer_owner.public_key().to_hex().as_str()]),
                    tag_values(["key_purpose", "app"]),
                    tag_values(["key_capability", "admin"]),
                    tag_values(["key_capability", "write"]),
                    tag_values(["key_added_at", "10"]),
                ],
                10,
            ),
            nostr_identity_roster_event(
                &peer_owner,
                vec![
                    tag_values(["op", "add_key"]),
                    tag_values(["key_pubkey", peer_device.public_key().to_hex().as_str()]),
                    tag_values(["key_purpose", "app"]),
                    tag_values(["key_capability", "write"]),
                    tag_values(["key_added_at", "11"]),
                ],
                11,
            ),
            nostr_identity_roster_event(
                &peer_owner,
                vec![
                    tag_values(["op", "tombstone_key"]),
                    tag_values(["target_pubkey", peer_device.public_key().to_hex().as_str()]),
                ],
                12,
            ),
        ] {
            engine
                .ingest_nostr_identity_roster_op_event(&event)
                .expect("roster fact");
        }

        let devices = engine.known_device_identity_pubkeys_for_owner(peer_owner.public_key());
        assert!(devices.is_empty());
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
            &[admin.public_key(), member.public_key()],
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
        assert_eq!(installed.members.len(), 2);
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
            &[admin.public_key()],
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

    fn has_filter_with_kind_author(filters: &[Filter], kind: u32, author: PublicKey) -> bool {
        let author_hex = author.to_hex();
        filters
            .iter()
            .map(|filter| serde_json::to_value(filter).expect("filter json"))
            .any(|filter| {
                let has_kind = filter
                    .get("kinds")
                    .and_then(|kinds| kinds.as_array())
                    .is_some_and(|kinds| {
                        kinds
                            .iter()
                            .any(|value| value.as_u64() == Some(kind as u64))
                    });
                let has_author = filter
                    .get("authors")
                    .and_then(|authors| authors.as_array())
                    .is_some_and(|authors| {
                        authors
                            .iter()
                            .any(|value| value.as_str() == Some(author_hex.as_str()))
                    });
                has_kind && has_author
            })
    }

    fn nostr_identity_roster_event(
        signer: &Keys,
        facts: Vec<Vec<String>>,
        created_at: u64,
    ) -> Event {
        const PROFILE_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
        let created_at_string = created_at.to_string();
        let signer_hex = signer.public_key().to_hex();
        let nonce = format!("nonce-{created_at}");
        let mut tags = vec![
            nostr::Tag::parse(["i", PROFILE_ID, "subject"]).expect("profile tag"),
            nostr::Tag::parse(["type", "nostr_identity_roster_op"]).expect("type tag"),
            nostr::Tag::parse(["schema", "1"]).expect("schema tag"),
            nostr::Tag::parse(["actor_pubkey", signer_hex.as_str()]).expect("actor tag"),
            nostr::Tag::parse(["client_nonce", nonce.as_str()]).expect("nonce tag"),
            nostr::Tag::parse(["created_at", created_at_string.as_str()]).expect("created_at tag"),
        ];
        for fact in facts {
            let values = fact.iter().map(String::as_str).collect::<Vec<_>>();
            tags.push(nostr::Tag::parse(values).expect("fact tag"));
        }
        nostr::EventBuilder::new(Kind::from(NOSTR_IDENTITY_ROSTER_OP_KIND as u16), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(signer)
            .expect("signed roster event")
    }

    fn tag_values<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(ToString::to_string).collect()
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
