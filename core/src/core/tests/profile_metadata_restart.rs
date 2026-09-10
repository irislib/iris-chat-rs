fn profile_restart_core(
    data_dir: &std::path::Path,
    relay: &crate::local_relay::TestRelay,
) -> (AppCore, flume::Receiver<CoreMsg>) {
    let (tx, rx) = flume::unbounded();
    let mut core = AppCore::new(
        flume::unbounded().0,
        tx,
        data_dir.to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.device_approval_relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    (core, rx)
}

fn wait_for_profile_restart(
    core: &mut AppCore,
    rx: &flume::Receiver<CoreMsg>,
    ready: impl Fn(&AppCore) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(message) = rx.recv_timeout(Duration::from_millis(20)) {
            core.handle_message(message);
        }
        if ready(core) {
            return;
        }
    }
    panic!("profile startup did not finish within 10 seconds");
}

fn profile_publish_finished(core: &AppCore) -> bool {
    core.debug_log.iter().any(|entry| {
        entry.category == "publish.identity" && entry.detail == "label=metadata success=true"
    })
}

fn newest_profile_on_relay(
    runtime: &tokio::runtime::Runtime,
    relay: &crate::local_relay::TestRelay,
    owner: PublicKey,
) -> Event {
    runtime.block_on(async {
        let reader = Client::default();
        let urls = relay_urls_from_strings(&[relay.url().to_string()]);
        ensure_session_relays_configured(&reader, &urls).await;
        connect_client_with_timeout(&reader, Duration::from_secs(2)).await;
        let events = reader
            .fetch_events(
                Filter::new().kind(Kind::Metadata).author(owner),
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        reader.disconnect().await;
        events.first_owned().expect("remote profile")
    })
}

fn assert_remote_profile_fields(core: &AppCore, owner: &Keys) {
    let profile = &core.owner_profiles[&owner.public_key().to_hex()];
    assert_eq!(profile.name.as_deref(), Some("Remote name"));
    assert_eq!(profile.display_name.as_deref(), Some("Remote display"));
    assert_eq!(
        profile.picture.as_deref(),
        Some("https://example.com/remote.png")
    );
    assert_eq!(profile.about.as_deref(), Some("Remote about"));
    let extra: serde_json::Value = serde_json::from_str(&profile.extra_metadata_json).unwrap();
    assert_eq!(
        extra["custom"],
        serde_json::json!({"nested": [1, true, "keep"]})
    );
    assert_eq!(extra["website"], "https://example.com");
    assert_eq!(profile.extra_tags, vec![vec!["alt", "Remote profile"]]);
}

#[test]
fn profile_metadata_restart_preserves_newer_remote_fields() {
    let relay = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let device = Keys::generate();
    let now = unix_now().get();
    let stale = EventBuilder::new(Kind::Metadata, r#"{"name":"Cached name"}"#)
        .custom_created_at(Timestamp::from_secs(now - 120))
        .sign_with_keys(&owner)
        .unwrap();
    let remote = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({
            "name": "Remote name", "display_name": "Remote display",
            "picture": "https://example.com/remote.png", "about": "Remote about",
            "website": "https://example.com", "custom": {"nested": [1, true, "keep"]}
        })
        .to_string(),
    )
    .tag(nostr::Tag::parse(["alt", "Remote profile"]).unwrap())
    .custom_created_at(Timestamp::from_secs(now - 60))
    .sign_with_keys(&owner)
    .unwrap();

    // Persist a real received profile, then stop Iris before another client edits it.
    let (mut core, _) = profile_restart_core(dir.path(), &relay);
    core.start_primary_session(owner.clone(), device.clone(), false, false)
        .unwrap();
    core.handle_relay_event(stale);
    assert!(core
        .owner_picture_url(&owner.public_key().to_hex())
        .is_none());
    core.shutdown();
    drop(core);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let publisher = Client::new(owner.clone());
        let urls = relay_urls_from_strings(&[relay.url().to_string()]);
        ensure_session_relays_configured(&publisher, &urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher.send_event(&remote).await.unwrap();
        publisher.disconnect().await;
    });
    assert!(relay_events(&relay)
        .iter()
        .any(|event| event.id == remote.id));

    let (mut core, rx) = profile_restart_core(dir.path(), &relay);
    core.restore_account_bundle(
        Some(owner.secret_key().to_secret_hex()),
        &owner.public_key().to_hex(),
        &device.secret_key().to_secret_hex(),
    );
    wait_for_profile_restart(&mut core, &rx, profile_publish_finished);
    assert_eq!(
        newest_profile_on_relay(&runtime, &relay, owner.public_key()).content,
        remote.content,
        "a fresh client must still see the remote profile after restart"
    );
    let destructive = relay_events(&relay)
        .into_iter()
        .filter(|event| {
            event.kind == Kind::Metadata
                && event.pubkey == owner.public_key()
                && event.created_at > remote.created_at
        })
        .collect::<Vec<_>>();
    assert!(
        destructive.is_empty(),
        "restart published {} newer metadata replacements from stale cache",
        destructive.len()
    );
    wait_for_profile_restart(&mut core, &rx, |core| {
        core.owner_picture_url(&owner.public_key().to_hex())
            .as_deref()
            == Some("https://example.com/remote.png")
    });
    assert_remote_profile_fields(&core, &owner);
    core.shutdown();
    drop(core);

    // Signing in with the secret key must also keep the refreshed persisted fields.
    let (mut core, rx) = profile_restart_core(dir.path(), &relay);
    core.restore_primary_session(&owner.secret_key().to_secret_hex());
    wait_for_profile_restart(&mut core, &rx, profile_publish_finished);
    assert_remote_profile_fields(&core, &owner);
    assert_eq!(
        newest_profile_on_relay(&runtime, &relay, owner.public_key()).content,
        remote.content,
        "signing in must not replace the remote profile"
    );
    core.handle_action(AppAction::UpdateProfileMetadata {
        name: "Intentional edit".to_string(),
        picture_url: Some("https://example.com/local.png".to_string()),
        about: Some("Local about".to_string()),
    });
    wait_for_profile_restart(&mut core, &rx, |_| {
        relay_events(&relay).iter().any(|event| {
            event.kind == Kind::Metadata
                && event.created_at > remote.created_at
                && event.content.contains("Intentional edit")
        })
    });
    let edited = relay_events(&relay)
        .into_iter()
        .find(|event| event.kind == Kind::Metadata && event.content.contains("Intentional edit"))
        .unwrap();
    let content: serde_json::Value = serde_json::from_str(&edited.content).unwrap();
    assert_eq!(content["picture"], "https://example.com/local.png");
    assert_eq!(content["about"], "Local about");
    assert_eq!(content["website"], "https://example.com");
    assert_eq!(
        content["custom"],
        serde_json::json!({"nested": [1, true, "keep"]})
    );
    assert_eq!(edited.tags, remote.tags);
    core.shutdown();
    drop(core);

    let (mut core, rx) = profile_restart_core(dir.path(), &relay);
    core.restore_primary_session(&owner.secret_key().to_secret_hex());
    wait_for_profile_restart(&mut core, &rx, profile_publish_finished);
    let profile = &core.owner_profiles[&owner.public_key().to_hex()];
    assert_eq!(profile.name.as_deref(), Some("Intentional edit"));
    assert_eq!(
        profile.picture.as_deref(),
        Some("https://example.com/local.png")
    );
    assert_eq!(profile.updated_at_secs, edited.created_at.as_secs());
    assert_eq!(
        newest_profile_on_relay(&runtime, &relay, owner.public_key()).id,
        edited.id
    );
    core.shutdown();
}

#[test]
fn profile_metadata_cached_self_is_refreshed_on_sign_in() {
    let relay = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _) = profile_restart_core(dir.path(), &relay);
    core.start_primary_session(owner.clone(), device.clone(), false, false)
        .unwrap();
    core.set_local_profile_name("Cached name");
    core.shutdown();
    drop(core);

    let (mut core, _rx) = profile_restart_core(dir.path(), &relay);
    core.restore_account_bundle(
        Some(owner.secret_key().to_secret_hex()),
        &owner.public_key().to_hex(),
        &device.secret_key().to_secret_hex(),
    );
    assert!(
        core.profile_metadata_fetch_inflight
            .contains(&owner.public_key().to_hex()),
        "cached self metadata must not skip refresh on sign in"
    );
    core.shutdown();
}

#[test]
fn profile_metadata_intentional_edit_advances_cached_timestamp() {
    let relay = crate::local_relay::TestRelay::start_with_bind("127.0.0.1:0").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let (mut core, rx) = profile_restart_core(dir.path(), &relay);
    core.start_primary_session(owner.clone(), Keys::generate(), false, false)
        .unwrap();
    let previous = EventBuilder::new(Kind::Metadata, r#"{"name":"Remote name","custom":42}"#)
        .custom_created_at(Timestamp::from_secs(unix_now().get() + 10))
        .sign_with_keys(&owner)
        .unwrap();
    core.handle_relay_event(previous.clone());
    core.handle_action(AppAction::UpdateProfileMetadata {
        name: "Intentional edit".to_string(),
        picture_url: None,
        about: None,
    });
    wait_for_profile_restart(&mut core, &rx, profile_publish_finished);
    let edited = relay_events(&relay)
        .into_iter()
        .find(|event| event.kind == Kind::Metadata && event.content.contains("Intentional edit"))
        .unwrap();
    assert!(edited.created_at > previous.created_at);
    assert_eq!(
        edited.created_at.as_secs(),
        core.owner_profiles[&owner.public_key().to_hex()].updated_at_secs
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&edited.content).unwrap()["custom"],
        42
    );
    core.handle_action(AppAction::DeleteProfileMetadata);
    wait_for_profile_restart(&mut core, &rx, |_| {
        relay_events(&relay).iter().any(|event| {
            event.kind == Kind::Metadata
                && event.content == "{}"
                && event.created_at > edited.created_at
        })
    });
    core.shutdown();
}
