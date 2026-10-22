mod full_invites {
    use super::*;
    use nostr::JsonUtil;

    fn assert_handshake_filter(core: &AppCore, invite: &Invite, owner: &Keys, peers: &[PublicKey]) {
        use nostr::{ClientMessage, SubscriptionId};
        let ephemeral = invite.inviter_ephemeral_public_key.to_string();
        let filters = core.recent_protocol_filters(unix_now());
        let selected = filters
            .into_iter()
            .filter(|filter| {
                serde_json::to_value(filter).unwrap()["#p"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(&ephemeral)))
            })
            .collect::<Vec<_>>();
        assert!(!selected.is_empty(), "missing durable invite subscription");
        let wire = ClientMessage::req(SubscriptionId::new("full-invite"), selected).as_json();
        let frame: serde_json::Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(frame[0], "REQ");
        for filter in frame.as_array().unwrap().iter().skip(2) {
            assert!(filter.get("authors").is_none());
            assert_eq!(filter["kinds"], serde_json::json!([INVITE_RESPONSE_KIND]));
        }
        assert!(!wire.contains(&owner.public_key().to_hex()));
        // Existing receiving scopes include this device's delivery key alongside
        // its ephemeral invite keys. They must never contain peer identities.
        for peer in peers {
            assert!(!wire.contains(&peer.to_hex()));
        }
        assert!(!wire.contains(&hashtree_core::to_hex(&invite.shared_secret)));
    }

    #[test]
    fn full_invite_creation_is_local_current_and_survives_restart() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let peers = [peer_owner.public_key(), peer_device.public_key()];
        let (mut core, updates, directory) =
            logged_in_test_core_with_updates("full-invite", &owner, &device);
        core.preferences.nostr_relay_urls.clear();
        core.pending_relay_publishes.clear();
        assert!(core.app_keys.is_empty(), "exercise missing local roster");
        let deferred = core.defer_owner_app_keys_publish;
        let lookup = core.owner_registration_lookup_generation;
        updates.try_iter().for_each(drop);

        core.handle_action(AppAction::CreateFullInvite);
        assert!(core.state.toast.is_none());
        assert!(core.pending_invite_upload.is_none());
        assert!(!core.state.busy.creating_invite);
        assert_eq!(core.defer_owner_app_keys_publish, deferred);
        assert_eq!(core.owner_registration_lookup_generation, lookup);
        assert!(core
            .app_store
            .load_pending_relay_publishes(&owner.public_key().to_hex())
            .unwrap()
            .is_empty());
        assert!(
            core.pending_relay_publishes.is_empty(),
            "full creation queued publication"
        );
        assert!(
            !updates
                .try_iter()
                .any(|update| matches!(update, AppUpdate::NearbyPublishedEvent { .. })),
            "full creation emitted a nearby publication"
        );
        let snapshot = core.state.public_invite.as_ref().unwrap();
        let url = snapshot.url.clone();
        assert!(url.starts_with("https://chat.iris.to/#/invite/"));
        assert_eq!(url, snapshot.legacy_url);
        let invite = super::super::invites::parse_public_invite_input(&url).unwrap();
        assert_eq!(invite.owner_public_key, Some(owner.public_key()));
        assert_eq!(invite.purpose.as_deref(), Some("private"));
        assert!(core.app_keys.contains_key(&owner.public_key().to_hex()));
        let issued = core.issued_invites.values().next().unwrap();
        assert!(issued.full && !issued.published && !issued.revoked);
        assert_eq!(
            issued.record.invite.version,
            nostr_double_ratchet::METADATA_PRIVACY_VERSION
        );
        issued.record.validate().unwrap();
        assert_handshake_filter(&core, &invite, &owner, &peers);
        let storage = core.private_chat_invite_storage().unwrap();
        let restored = super::super::issued_invites::load_issued_invites(&storage).unwrap();
        assert_eq!(restored.values().next().unwrap().url, url);
        drop(storage);
        drop(core);

        // Reopen through the production account restoration path, not a copied map.
        let mut restarted = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            directory.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        restarted.preferences.nostr_relay_urls.clear();
        restarted
            .start_primary_session(owner.clone(), device.clone(), true, true)
            .unwrap();
        assert_eq!(restarted.personal_invite_snapshot().unwrap().url, url);
        assert!(restarted
            .app_keys
            .contains_key(&owner.public_key().to_hex()));
        assert_handshake_filter(&restarted, &invite, &owner, &peers);
        let response = crafted_private_invite_response(
            &url,
            &peer_device,
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        );
        let wire = nostr::ClientMessage::event(response).as_json();
        let frame: serde_json::Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(frame[0], "EVENT");
        let event = Event::from_json(frame[1].to_string()).unwrap();
        assert!(nostr_double_ratchet::has_metadata_privacy_version(&event));
        assert_eq!(
            event
                .tags
                .public_keys()
                .map(|key| key.to_hex())
                .collect::<Vec<_>>(),
            vec![invite.inviter_ephemeral_public_key.to_string()]
        );
        assert_ne!(event.pubkey, peer_owner.public_key());
        assert_ne!(event.pubkey, peer_device.public_key());
        assert!(!wire.contains(&owner.public_key().to_hex()));
        assert!(!wire.contains(&device.public_key().to_hex()));
        assert!(!wire.contains(&peer_owner.public_key().to_hex()));
        assert!(!wire.contains(&peer_device.public_key().to_hex()));
        restarted.handle_relay_event(event);
        prove_invite_owner(&mut restarted, &peer_owner, &peer_device, 10);
        assert_eq!(stored_session_count(&restarted, peer_owner.public_key()), 1);
    }

    #[test]
    fn full_invite_preserves_siblings_when_deferred_registration_completes() {
        let (mut core, owner, device) = deferred_registration_core("full-deferred-registration");
        let sibling = Keys::generate();
        let remote = AppKeys::new(vec![DeviceEntry::new(sibling.public_key(), 10)])
            .get_event_at(owner.public_key(), 10)
            .sign_with_keys(&owner)
            .unwrap();
        let lookup = core.owner_registration_lookup_generation;
        core.handle_action(AppAction::CreateFullInvite);
        assert!(core.state.toast.is_none());
        assert!(core.state.public_invite.is_some());
        assert!(core.defer_owner_app_keys_publish);
        assert_eq!(core.owner_registration_lookup_generation, lookup);
        assert!(!core.app_keys.contains_key(&owner.public_key().to_hex()));
        assert!(core.pending_relay_publishes.is_empty());
        // Full creation must not introduce an authoritative singleton roster or
        // make one eligible for later generic identity publication.
        assert!(core
            .build_local_identity_artifacts()
            .1
            .iter()
            .all(|(label, _)| *label != "app-keys"));
        core.complete_owner_registration_lookup(
            core.relay_status_watch_generation,
            owner.public_key(),
            device.public_key(),
            1,
            1,
            vec![remote],
        );
        assert!(!core.defer_owner_app_keys_publish);
        let known = core.app_keys.get(&owner.public_key().to_hex()).unwrap();
        let recovered = known_app_keys_to_ndr(known);
        assert_eq!(known.devices.len(), 2);
        assert!(recovered.get_device(&sibling.public_key()).is_some());
        assert!(recovered.get_device(&device.public_key()).is_some());
        // Only normal completed recovery now publishes the merged roster.
        let publications = core
            .pending_relay_publishes
            .values()
            .filter(|pending| pending.label == "app-keys")
            .collect::<Vec<_>>();
        assert_eq!(publications.len(), 1);
        let published = Event::from_json(&publications.first().unwrap().event_json).unwrap();
        published.verify().unwrap();
        let roster = AppKeys::from_event(&published).unwrap();
        assert_eq!(roster.get_all_devices().len(), 2);
        assert!(roster.get_device(&sibling.public_key()).is_some());
        assert!(roster.get_device(&device.public_key()).is_some());
    }

    #[test]
    fn full_marker_cannot_publish_or_admit_a_group_invite() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut core = logged_in_test_core("full-group-denied", &owner, &device);
        core.create_group("Group", &[]);
        let group = core.groups.keys().next().unwrap().clone();
        let (key, mut issued) = core
            .prepare_issued_invite(Some(&group), None, None)
            .unwrap();
        issued.full = true;
        assert!(!issued.published);
        core.issued_invites.insert(key, issued.clone());
        assert_eq!(
            core.admit_invite_request(&issued, Keys::generate().public_key())
                .unwrap(),
            "unavailable"
        );
        assert!(core.group_invite_snapshots(&group).is_empty());
        assert!(core.personal_invite_snapshot().is_none());
    }

    #[test]
    fn full_invite_requires_durable_storage_before_exposing_a_link() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut core = logged_in_test_core("full-storage-denied", &owner, &device);
        core.app_store
            .shared()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TEMP TRIGGER reject_full_invite BEFORE INSERT ON ndr_kv
             WHEN NEW.key LIKE 'issued-private-invites-v1/%'
               AND json_extract(NEW.value, '$.full') = 1
             BEGIN SELECT RAISE(ABORT, 'injected storage failure'); END;",
            )
            .unwrap();
        core.handle_action(AppAction::CreateFullInvite);
        assert!(core.state.toast.is_some());
        assert!(core.state.public_invite.is_none());
        assert!(core.pending_invite_upload.is_none());
        assert_eq!(core.private_chat_invites.len(), 1);
        assert!(core
            .issued_invites
            .values()
            .all(|issued| !issued.full && !issued.published));
    }
}
