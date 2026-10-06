#[test]
fn nostrconnect_device_link_preserves_private_history_choice_and_external_signer_boundary() {
    for include_history in [false, true] {
        let relay = crate::local_relay::TestRelay::start();
        let missing_ack = crate::local_relay::TestRelay::start();
        missing_ack.ignore_acknowledgements(24133).unwrap();
        let owner = Keys::generate();
        let approver = Keys::generate();
        let (mut source, _, _source_dir) =
            logged_in_test_core_with_updates("nip46-approver", &owner, &approver);
        let (source_tx, source_messages) = flume::unbounded();
        source.core_sender = source_tx;
        let mut relays = vec![relay.url().to_string()];
        if include_history {
            // A default server being down must not prevent phone-to-browser linking.
            relays.push("ws://127.0.0.1:1".into());
            // Connected-but-silent publication differs from connection refusal.
            relays.push(missing_ack.url().to_string());
        }
        source.preferences.nostr_relay_urls = relays.clone();
        source.preferences.nearby_enabled = false;
        source.logged_in.as_mut().unwrap().relay_urls = relays
            .iter().map(|url| RelayUrl::parse(url).unwrap()).collect();
        let old = AppKeys::new(vec![DeviceEntry::new(
            approver.public_key(),
            unix_now().get() - 10,
        )])
        .get_event_at(owner.public_key(), unix_now().get() - 2)
        .sign_with_keys(&owner)
        .unwrap();
        source.app_keys.insert(
            owner.public_key().to_hex(),
            known_app_keys_from_ndr(
                owner.public_key(),
                &AppKeys::from_event(&old).unwrap(),
                old.created_at.as_secs(),
            ),
        );
        publish_signer_test_event(&source, &relay, &old);
        if include_history {
            let mut legacy = AppKeys::from_event(&old).unwrap()
                .get_event_at(owner.public_key(), old.created_at.as_secs());
            legacy.tags.push(nostr::Tag::parse([
                nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT,
                "retired-label-ciphertext",
            ]).unwrap());
            legacy.id = None;
            let duplicate = legacy.sign_with_keys(&owner).unwrap();
            assert_ne!(duplicate.id, old.id);
            publish_signer_test_event(&source, &relay, &duplicate);
            assert_eq!(source.runtime.block_on(super::account_signer_relay::fetch_signer_roster_heads(
                owner.public_key(), &[RelayUrl::parse(relay.url()).unwrap()],
            )).unwrap().len(), 2, "approval preparation sees every distinct authenticated head");
        }
        let target_dir = tempfile::TempDir::new().unwrap();
        let (mut target, target_messages, _) =
            signer_test_core(target_dir.path(), relays);
        target.handle_action(AppAction::StartRemoteSignerLogin);
        pump_signer_core_until(&mut target, &target_messages, |core| {
            core.state
                .remote_signer_login
                .as_ref()
                .is_some_and(|state| state.phase == crate::RemoteSignerPhase::WaitingForSigner)
        });
        let uri = target
            .state
            .remote_signer_login
            .as_ref()
            .unwrap()
            .connection_uri
            .clone()
            .unwrap();
        let link_id = url::Url::parse(&uri)
            .unwrap()
            .host_str()
            .unwrap()
            .to_string();
        source.handle_action(AppAction::AddAuthorizedDeviceWithHistory {
            device_input: format!("  {uri}\n"),
            include_message_history: include_history,
        });
        assert!(source.pending_device_link_signer.is_some());
        let deadline = Instant::now() + Duration::from_secs(100);
        while target.logged_in.is_none() && target.state.busy.restoring_session {
            for message in source_messages.try_iter() {
                source.handle_message(message);
            }
            assert_ne!(source.state.toast.as_deref(), Some("Could not link device. Try again."));
            if let Ok(message) = target_messages.recv_timeout(Duration::from_millis(10)) {
                target.handle_message(message);
            }
            assert!(
                Instant::now() < deadline,
                "source={:?} target={:?}",
                source.state.toast,
                target.state.toast
            );
        }
        assert!(
            target.logged_in.is_some(),
            "source={:?} target={:?}",
            source.state.toast,
            target.state.toast
        );
        let device = target.logged_in.as_ref().unwrap().device_keys.public_key();
        assert!(target.logged_in.as_ref().unwrap().owner_keys.is_none());
        let paired = target
            .device_history_transfer(&approver.public_key().to_hex())
            .unwrap();
        assert_eq!(paired.link_id, link_id);
        assert!(!paired.outbound);
        assert!(
            !paired.policy_known,
            "history policy travels only on private device sync"
        );
        let signed = relay
            .events()
            .iter()
            .filter_map(|value| serde_json::from_value::<Event>(value.clone()).ok())
            .filter(|event| event.kind.as_u16() == APP_KEYS_EVENT_KIND as u16)
            .max_by_key(|event| event.created_at)
            .unwrap();
        assert!(signed.content.is_empty());
        assert!(!serde_json::to_string(&signed).unwrap().contains("history"));
        assert!(!serde_json::to_string(&signed).unwrap().contains("encrypted_device_labels"));
        source.apply_app_keys_event(&signed).unwrap();
        let outgoing = source.device_history_transfer(&device.to_hex()).unwrap();
        assert_eq!(outgoing.link_id, paired.link_id);
        assert_eq!(outgoing.link_at, paired.link_at);
        assert_eq!(
            outgoing.since,
            if include_history { 0 } else { paired.link_at }
        );
        assert_eq!(outgoing.complete, !include_history);
        source.stop_device_link_signer();
    }
}

#[test]
fn nostrconnect_device_approval_is_one_addition_with_immutable_devices_and_bounded_uri() {
    use super::device_link_signer::validate_device_link_draft;
    use super::remote_signer_uri::parse_device_link_connection;
    let owner = Keys::generate();
    let existing = Keys::generate();
    let target = Keys::generate();
    let now = unix_now().get();
    let old = AppKeys::new(vec![DeviceEntry::new(existing.public_key(), now - 20)])
        .get_event_at(owner.public_key(), now - 1)
        .sign_with_keys(&owner)
        .unwrap();
    let draft =
        prepare_signer_authorization(owner.public_key(), target.public_key(), Some(&old), now)
            .unwrap();
    assert!(
        draft
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", &owner.public_key().to_hex()]),
        "browser canonical snapshots retain the owner index"
    );
    assert_eq!(
        validate_device_link_draft(owner.public_key(), &draft, Some(&old), now).unwrap(),
        (target.public_key(), now)
    );
    for field in [
        "remove",
        "change_join",
        "extra",
        "kind",
        "content",
        "another_device",
        "future",
    ] {
        let mut changed = draft.clone();
        match field {
            "remove" => {
                changed.tags = AppKeys::new(vec![DeviceEntry::new(target.public_key(), now)])
                    .get_event_at(owner.public_key(), now)
                    .tags
            }
            "change_join" => {
                changed.tags = AppKeys::new(vec![
                    DeviceEntry::new(existing.public_key(), now),
                    DeviceEntry::new(target.public_key(), now),
                ])
                .get_event_at(owner.public_key(), now)
                .tags
            }
            "extra" => changed
                .tags
                .push(nostr::Tag::parse(["extra", "private"]).unwrap()),
            "kind" => changed.kind = Kind::TextNote,
            "content" => changed.content = "history should never be public".into(),
            "another_device" => changed.tags.push(
                nostr::Tag::parse([
                    "device".to_string(),
                    Keys::generate().public_key().to_hex(),
                    now.to_string(),
                ])
                .unwrap(),
            ),
            "future" => changed.created_at = Timestamp::from(now + 301),
            _ => unreachable!(),
        }
        assert!(
            validate_device_link_draft(owner.public_key(), &changed, Some(&old), now).is_err(),
            "accepted {field}"
        );
    }
    let key = Keys::generate().public_key().to_hex();
    assert!(parse_device_link_connection(&format!(
        "nostrconnect://{key}?relay=wss://example.com&secret=abc&perms=sign_event:37368"
    ))
    .is_ok());
    for suffix in [
        "relay=wss://example.com",
        "relay=https://example.com&secret=abc",
        "relay=wss://example.com&secret=a&secret=b",
        "relay=wss://example.com&secret=a&perms=sign_event:1",
    ] {
        assert!(parse_device_link_connection(&format!("nostrconnect://{key}?{suffix}")).is_err());
    }
}

#[test]
fn nostrconnect_approval_replay_is_idempotent_and_cancellation_revokes_signing() {
    let owner = Keys::generate();
    let approver = Keys::generate();
    let target = Keys::generate();
    let client = Keys::generate();
    let now = unix_now().get();
    let relay = crate::local_relay::TestRelay::start();
    let (mut source, _, _dir) = logged_in_test_core_with_updates("nip46-replay", &owner, &approver);
    let old = AppKeys::new(vec![DeviceEntry::new(approver.public_key(), now - 20)])
        .get_event_at(owner.public_key(), now - 1)
        .sign_with_keys(&owner)
        .unwrap();
    source.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::from_event(&old).unwrap(),
            old.created_at.as_secs(),
        ),
    );
    let uri = super::remote_signer_uri::client_connection_uri(
        &client,
        &[RelayUrl::parse(relay.url()).unwrap()],
        "private-challenge",
    );
    source.handle_action(AppAction::AddAuthorizedDeviceWithHistory {
        device_input: uri,
        include_message_history: true,
    });
    let token = source
        .pending_device_link_signer
        .as_ref()
        .unwrap()
        .token
        .clone();
    let draft =
        prepare_signer_authorization(owner.public_key(), target.public_key(), Some(&old), now)
            .unwrap();
    let mut standard = serde_json::to_value(&draft).unwrap();
    standard.as_object_mut().unwrap().remove("pubkey");
    let json = standard.to_string();
    let (signed, info) = source
        .sign_device_link_request(&token, &json, Some(&old))
        .unwrap();
    let (replayed, _) = source
        .sign_device_link_request(&token, &json, Some(&old))
        .unwrap();
    assert_eq!(signed.id, replayed.id);
    assert_eq!(info.device, target.public_key().to_hex());
    assert_eq!(info.link_id, client.public_key().to_hex());
    let changed = prepare_signer_authorization(
        owner.public_key(),
        Keys::generate().public_key(),
        Some(&old),
        now,
    )
    .unwrap();
    assert!(source
        .sign_device_link_request(
            &token,
            &serde_json::to_string(&changed).unwrap(),
            Some(&old)
        )
        .is_err());
    assert!(
        source
            .device_history_transfer(&target.public_key().to_hex())
            .is_none(),
        "unsigned/unpublished target cannot receive history"
    );
    source.finish_device_link_signer(&token, true);
    assert!(
        source.state.busy.updating_roster,
        "a returned signature is not a completed link"
    );
    assert_ne!(source.state.toast.as_deref(), Some("Device added"));
    source.apply_app_keys_event(&signed).unwrap();
    assert!(
        !source.state.busy.updating_roster,
        "observing the published authorization completes linking"
    );
    assert_eq!(source.state.toast.as_deref(), Some("Device added"));
    source.stop_device_link_signer();
    assert!(source
        .sign_device_link_request(&token, &json, Some(&old))
        .is_err());
}
