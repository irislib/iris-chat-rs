fn local_roster_artifact(core: &AppCore) -> Event {
    core.build_local_identity_artifacts()
        .1
        .into_iter()
        .map(|(_, event)| event)
        .find(is_app_keys_event)
        .expect("published roster")
}

#[test]
fn unchanged_local_roster_republication_keeps_one_durable_signed_head() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let directory = tempfile::TempDir::new().unwrap();
    let data_dir = directory.path().to_string_lossy().to_string();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir.clone());
    let known = known_app_keys_from_ndr(owner.public_key(), &AppKeys::new(vec![
        DeviceEntry::new(device.public_key(), unix_now().get()),
    ]), unix_now().get());
    core.app_keys.insert(owner.public_key().to_hex(), known.clone());
    let first = local_roster_artifact(&core);
    assert_eq!(local_roster_artifact(&core), first, "unchanged reads reuse the signed roster");
    core.handle_action(AppAction::CreateChat { peer_input: Keys::generate().public_key().to_hex() });
    core.handle_action(AppAction::CreateChat { peer_input: owner.public_key().to_hex() });
    core.send_message(&owner.public_key().to_hex(), "an old note", None);
    core.publish_local_app_keys();
    core.publish_local_app_keys_snapshot_only("test_roster_republish");
    assert_eq!(local_roster_artifact(&core), first, "chat activity does not create a conflicting head");
    for pending in core.pending_relay_publishes.values().filter(|pending| pending.label == "app-keys") {
        let event: Event = serde_json::from_str(&pending.event_json).unwrap();
        assert_eq!(event.id, first.id);
    }
    core.persist_best_effort_inner();
    drop(core);
    let mut restarted = logged_in_test_core_at_data_dir(&owner, &device, data_dir);
    restarted.app_keys.insert(owner.public_key().to_hex(), known);
    assert_eq!(local_roster_artifact(&restarted), first, "restarting retains the exact signed head");
}

#[test]
fn changed_local_roster_requires_new_revision_and_reuses_imported_signed_head() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let core = logged_in_test_core("signed-roster-revision", &owner, &device);
    let now = unix_now().get();
    let mut roster = AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)]);
    let first = core.signed_local_app_keys_snapshot(&roster, now, &owner).unwrap();
    roster.add_device(DeviceEntry::new(sibling.public_key(), now));
    assert!(core.signed_local_app_keys_snapshot(&roster, now, &owner).is_err());
    let next = core.signed_local_app_keys_snapshot(&roster, now + 1, &owner).unwrap();
    assert_ne!(next.id, first.id);
    assert_eq!(core.signed_local_app_keys_snapshot(&roster, now + 1, &owner).unwrap(), next);
    assert!(core.signed_local_app_keys_snapshot(&roster, now, &owner).is_err());
    let mut core = core;
    let imported = super::account_signer::canonical_signer_roster(&roster, owner.public_key(), now + 2)
        .sign_with_keys(&owner).unwrap();
    core.cache_local_fips_identity(&imported);
    assert_eq!(core.signed_local_app_keys_snapshot(&roster, now + 2, &owner).unwrap(), imported);
}

#[test]
fn nostrconnect_roster_repair_refuses_real_conflicts_and_replays_one_exact_head() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let client = Keys::generate();
    let now = unix_now().get();
    let roster = AppKeys::new(vec![DeviceEntry::new(device.public_key(), now - 20)]);
    let first = roster.get_event_at(owner.public_key(), now - 1).sign_with_keys(&owner).unwrap();
    let duplicate = roster.get_event_at(owner.public_key(), now - 1).sign_with_keys(&owner).unwrap();
    let mut core = logged_in_test_core("roster-repair", &owner, &device);
    core.app_keys.insert(owner.public_key().to_hex(), known_app_keys_from_ndr(owner.public_key(), &roster, now - 1));
    let relay = crate::local_relay::TestRelay::start();
    let uri = super::remote_signer_uri::client_connection_uri(&client,
        &[RelayUrl::parse(relay.url()).unwrap()], "repair-challenge");
    core.start_device_link_signer(&uri, false);
    let token = core.pending_device_link_signer.as_ref().unwrap().token.clone();
    let heads = vec![first.clone(), duplicate.clone()];
    let repaired = core.prepare_device_link_roster_repair(&token, &heads).unwrap();
    assert!(repaired.created_at > first.created_at);
    let template = heads.iter().min_by_key(|event| event.id).unwrap();
    assert_eq!(repaired.tags, template.tags);
    assert_eq!(repaired.content, template.content);
    assert_eq!(core.prepare_device_link_roster_repair(&token, &heads).unwrap(), repaired);
    for change in ["membership", "join", "extra", "index", "subject", "duplicate_subject", "content"] {
        let mut tags = duplicate.tags.clone().to_vec();
        let mut content = String::new();
        match change {
            "membership" => tags.push(nostr::Tag::parse(["device".into(), Keys::generate().public_key().to_hex(), now.to_string()]).unwrap()),
            "join" => {
                tags.retain(|tag| tag.as_slice().first().is_none_or(|key| key != "device"));
                tags.push(nostr::Tag::parse(["device".into(), device.public_key().to_hex(), now.to_string()]).unwrap());
            }
            "extra" => tags.push(nostr::Tag::parse(["extra", "value"]).unwrap()),
            "index" => tags.push(nostr::Tag::public_key(Keys::generate().public_key())),
            "subject" => {
                tags.retain(|tag| tag.as_slice().first().is_none_or(|key| key != "i"));
                tags.push(nostr::Tag::parse(["i", "not-the-profile", "subject"]).unwrap());
            }
            "duplicate_subject" => tags.push(nostr::Tag::parse(["d", &uuid::Uuid::new_v4().to_string()]).unwrap()),
            "content" => content = "unexpected".into(),
            _ => unreachable!(),
        }
        let changed = UnsignedEvent::new(owner.public_key(), duplicate.created_at, duplicate.kind, tags, content)
            .sign_with_keys(&owner).unwrap();
        assert!(core.prepare_device_link_roster_repair(&token, &[first.clone(), changed]).is_err(), "accepted {change}");
    }
    core.app_keys.get_mut(&owner.public_key().to_hex()).unwrap().created_at_secs = now + 1;
    assert!(core.prepare_device_link_roster_repair(&token, &heads).is_err(), "local edits made during network preparation invalidate cached repair");
    core.stop_device_link_signer();
    assert!(core.prepare_device_link_roster_repair(&token, &heads).is_err());
}
