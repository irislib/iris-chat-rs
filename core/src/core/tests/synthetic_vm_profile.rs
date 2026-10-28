// Export only freshly generated test state for the unchanged native app. The
// output contains synthetic secret keys and must stay private. Generation and
// any finite publication of relay-events.json are separate from CPU measurement.
mod synthetic_vm_profile {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::path::Path;

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Inputs {
        output: std::path::PathBuf,
        run_id: String,
        relay_urls: Vec<String>,
    }

    impl Inputs {
        fn validate(&self) -> anyhow::Result<()> {
            anyhow::ensure!(
                self.output.is_absolute() && !self.output.exists(),
                "fixture output must be a new absolute path"
            );
            let parent = self
                .output
                .parent()
                .ok_or_else(|| anyhow::anyhow!("output parent"))?;
            anyhow::ensure!(
                parent.is_dir() && parent.canonicalize()? == parent,
                "output parent must be canonical"
            );
            anyhow::ensure!(
                self.run_id.starts_with("synthetic-vm-")
                    && self.run_id.len() <= 96
                    && self
                        .run_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "a distinct synthetic-vm run ID is required"
            );
            anyhow::ensure!(
                self.relay_urls.len() == 5,
                "select exactly five TLS relay URLs"
            );
            let mut unique = HashSet::new();
            for relay in &self.relay_urls {
                let url = url::Url::parse(relay)?;
                anyhow::ensure!(
                    url.scheme() == "wss"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none(),
                    "fixture relays must be explicit TLS endpoints without credentials"
                );
                anyhow::ensure!(
                    unique.insert(normalize_nostr_relay_url(relay).map_err(anyhow::Error::msg)?),
                    "duplicate fixture relay"
                );
            }
            Ok(())
        }
    }

    fn private_json(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }

    fn file_binding(path: &Path) -> anyhow::Result<serde_json::Value> {
        let mut input = std::fs::File::open(path)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        Ok(
            serde_json::json!({"bytes": input.metadata()?.len(), "sha256": format!("{:x}", hash.finalize())}),
        )
    }

    fn inventory_profile(
        root: &Path,
    ) -> anyhow::Result<(serde_json::Map<String, serde_json::Value>, Vec<String>)> {
        let mut files = serde_json::Map::new();
        let mut directories = Vec::new();
        let mut pending = vec![std::path::PathBuf::new()];
        let mut entries = 0;
        let mut bytes = 0u64;
        while let Some(relative) = pending.pop() {
            let directory = root.join(&relative);
            anyhow::ensure!(
                std::fs::symlink_metadata(&directory)?.is_dir(),
                "invalid fixture directory: {}",
                relative.display()
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            }
            directories.push(if relative.as_os_str().is_empty() {
                "profile".to_owned()
            } else {
                format!("profile/{}", relative.display())
            });
            for entry in std::fs::read_dir(&directory)? {
                let entry = entry?;
                let name = relative.join(entry.file_name());
                entries += 1;
                anyhow::ensure!(
                    entries <= 256 && name.components().count() <= 8,
                    "fixture inventory bound exceeded: {}",
                    name.display()
                );
                anyhow::ensure!(
                    name.components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
                        && name.to_str().is_some_and(|s| s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_. /".contains(&b))),
                    "unexpected fixture path: {}",
                    name.display()
                );
                let kind = entry.file_type()?;
                anyhow::ensure!(
                    !kind.is_symlink() && (kind.is_dir() || kind.is_file()),
                    "unsupported fixture entry: {}",
                    name.display()
                );
                if kind.is_dir() {
                    anyhow::ensure!(
                        name.starts_with("attachments"),
                        "unexpected fixture directory: {}",
                        name.display()
                    );
                    pending.push(name);
                    continue;
                }
                anyhow::ensure!(
                    name.starts_with("attachments")
                        || matches!(
                            name.to_str(),
                            Some(
                                "account-secret.json"
                                    | "core.sqlite3"
                                    | "core.sqlite3-wal"
                                    | "core.sqlite3-shm"
                                    | "core.sqlite3-journal"
                                    | "core.lock.sqlite3"
                                    | "core.lock.sqlite3-journal"
                                    | "iris_chat_runtime_debug.json"
                            )
                        ),
                    "unexpected fixture file: {}",
                    name.display()
                );
                bytes = bytes
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| anyhow::anyhow!("fixture byte count overflow"))?;
                anyhow::ensure!(
                    bytes <= 512 * 1024 * 1024,
                    "fixture inventory exceeds 512 MiB: {}",
                    name.display()
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600))?;
                }
                files.insert(
                    format!("profile/{}", name.display()),
                    file_binding(&entry.path())?,
                );
            }
        }
        directories.sort();
        Ok((files, directories))
    }

    fn seed_corpus(events: Vec<Event>) -> anyhow::Result<(Vec<Event>, serde_json::Value)> {
        // Profile names stay in the generated local DB. Only the public
        // authorization/invite records are eligible for finite relay seeding.
        let events = events
            .into_iter()
            .filter(|event| event.kind != Kind::Metadata)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !events.is_empty() && events.len() <= 290,
            "synthetic seed event count exceeded"
        );
        for event in &events {
            event.verify()?;
            anyhow::ensure!(
                [APP_KEYS_EVENT_KIND, INVITE_EVENT_KIND].contains(&(event.kind.as_u16() as u32)),
                "fixture relay corpus may contain only app keys and invites"
            );
            let expires = event
                .tags
                .expiration()
                .ok_or_else(|| anyhow::anyhow!("synthetic seed expiration missing"))?
                .as_secs();
            anyhow::ensure!(
                expires > unix_now().get() && expires <= unix_now().get() + 24 * 60 * 60,
                "synthetic seed expiration must be within 24 hours"
            );
        }
        let public_events = serde_json::to_value(&events)?;
        anyhow::ensure!(
            serde_json::to_vec_pretty(&public_events)?.len() < 1024 * 1024,
            "synthetic seed corpus exceeds 1 MiB"
        );
        Ok((events, public_events))
    }

    fn export(
        inputs: &Inputs,
        mut fixture: SyntheticHistory,
        events: Vec<Event>,
        cache_only: &[(PublicKey, PublicKey)],
    ) -> anyhow::Result<serde_json::Value> {
        inputs.validate()?;
        let (events, public_events) = seed_corpus(events)?;
        let login = fixture
            .core
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("generated account missing"))?;
        let owner = login
            .owner_keys
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("generated owner missing"))?;
        // This is exactly Swift StoredAccountBundle's Codable schema. Never
        // print these values or derive them from an existing account/profile.
        let account = serde_json::json!({
            "ownerNsec": owner.secret_key().to_bech32()?,
            "ownerPubkeyHex": owner.public_key().to_hex(),
            "deviceNsec": login.device_keys.secret_key().to_bech32()?,
        });
        let plan = fixture
            .core
            .compute_protocol_subscription_plan()
            .ok_or_else(|| anyhow::anyhow!("generated discovery plan missing"))?;
        anyhow::ensure!(
            plan.group_roster_authors.is_empty()
                && plan.group_sender_key_authors.is_empty()
                && plan.group_roster_group_ids.is_empty(),
            "public group subscriptions forbidden"
        );
        let discovery_authors = plan
            .roster_authors
            .iter()
            .chain(&plan.invite_authors)
            .collect::<HashSet<_>>()
            .len();
        let checkpoint = fixture
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot();
        for (owner, device) in cache_only {
            let owner_hex = owner.to_hex();
            let device_hex = device.to_hex();
            anyhow::ensure!(
                fixture.core.app_keys.contains_key(&owner_hex),
                "cache-only roster was lost"
            );
            anyhow::ensure!(
                !plan.roster_authors.contains(&owner_hex)
                    && !plan.invite_authors.contains(&owner_hex)
                    && !plan.invite_authors.contains(&device_hex)
                    && !fixture.core.threads.contains_key(&owner_hex)
                    && checkpoint
                        .users
                        .iter()
                        .filter(|user| user.owner_pubkey == ndr_owner_pubkey(*owner))
                        .flat_map(|user| &user.devices)
                        .all(|device| device.active_session.is_none()
                            && device.inactive_sessions.is_empty()),
                "cache-only owner unexpectedly has live demand"
            );
        }
        let known_devices = fixture
            .core
            .app_keys
            .values()
            .map(|keys| keys.devices.len())
            .sum::<usize>();
        // The offline builder disables nearby transports while generating SQL.
        // Persist the real startup defaults for the unchanged native app; merely
        // changing these fields does not start/reconcile networking here.
        let defaults = PreferencesSnapshot::default();
        fixture.core.preferences.nearby_enabled = defaults.nearby_enabled;
        fixture.core.preferences.nearby_bluetooth_enabled = defaults.nearby_bluetooth_enabled;
        fixture.core.preferences.nearby_lan_enabled = defaults.nearby_lan_enabled;
        let transport_preferences = serde_json::json!({
            "nearbyEnabled": fixture.core.preferences.nearby_enabled,
            "nearbyBluetoothEnabled": fixture.core.preferences.nearby_bluetooth_enabled,
            "nearbyLanEnabled": fixture.core.preferences.nearby_lan_enabled,
            "nearbyMailbagEnabled": fixture.core.preferences.nearby_mailbag_enabled,
        });
        fixture.core.preferences.nostr_relay_urls = normalize_nostr_relay_urls(&inputs.relay_urls);
        fixture.core.persist_app_state()?;
        fixture.core.shutdown();
        drop(fixture.core);
        drop(fixture.updates);
        // Close the real database before moving its directory. No whole-profile
        // duplicate, secret rewrite, source profile clone, or journal trimming.
        let database_path = fixture.directory.path().join("core.sqlite3");
        let (messages, threads, groups, app_keys) = {
            let database = rusqlite::Connection::open(&database_path)?;
            database.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
            let integrity: String = database.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
            anyhow::ensure!(integrity == "ok", "generated database integrity failed");
            (
                database.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, u64>(0))?,
                database.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, u64>(0))?,
                database.query_row("SELECT count(*) FROM groups", [], |r| r.get::<_, u64>(0))?,
                database.query_row("SELECT count(*) FROM app_keys", [], |r| r.get::<_, u64>(0))?,
            )
        };
        anyhow::ensure!(
            messages == fixture.rows && threads == fixture.chat_ids.len() as u64,
            "generated history changed during close"
        );
        std::fs::create_dir(&inputs.output)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&inputs.output, std::fs::Permissions::from_mode(0o700))?;
        }
        private_json(
            &fixture.directory.path().join("account-secret.json"),
            &account,
        )?;
        std::fs::rename(fixture.directory.path(), inputs.output.join("profile"))?;
        let _ = fixture.directory.keep();
        private_json(&inputs.output.join("relay-events.json"), &public_events)?;
        let (mut files, directories) = inventory_profile(&inputs.output.join("profile"))?;
        files.insert(
            "relay-events.json".into(),
            file_binding(&inputs.output.join("relay-events.json"))?,
        );
        let receipt = serde_json::json!({
            "schema": "iris-synthetic-vm-profile/v1",
            "passed": true, "syntheticOnly": true, "productionProfileRead": false,
            "launchAuthorized": false, "publicationPerformed": false,
            "runID": inputs.run_id, "relayURLs": inputs.relay_urls,
            "transportPreferences": transport_preferences,
            "counts": {"messages": messages, "threads": threads, "groups": groups,
                "appKeys": app_keys, "discoveryAuthors": discovery_authors,
                "rosterAuthors": plan.roster_authors.len(), "inviteAuthors": plan.invite_authors.len(),
                "trackedContacts": threads - groups, "knownDevices": known_devices,
                "cacheOnlyOwners": cache_only.len(), "cacheOnlyDiscoveryAuthors": 0,
                "relayEvents": events.len()},
            "seedBounds": {"maxEvents": 290, "maxBytes": 1048576, "maxLifetimeSeconds": 86400,
                "kinds": [APP_KEYS_EVENT_KIND, INVITE_EVENT_KIND],
                "firstExpiryUTCUnix": events.iter().filter_map(|e| e.tags.expiration()).map(|t| t.as_secs()).min()},
            "files": files,
            "directories": directories,
            "fixtureSourceSHA256": format!("{:x}", Sha256::digest(include_bytes!("synthetic_history_scale.rs"))),
            "exporterSourceSHA256": format!("{:x}", Sha256::digest(include_bytes!("synthetic_vm_profile.rs"))),
            "launchEnvironment": {"IRIS_UI_TEST_RUN_ID": inputs.run_id,
                "IRIS_UI_TEST_BYPASS_KEYCHAIN": "1", "IRIS_DISABLE_NOTIFICATIONS": "1"},
            "requiredBeforeLaunch": ["exact candidate binary and guest path binding",
                "IRIS_UI_TEST_DATA_DIR bound to this profile in the guest",
                "bounded synthetic event publication and readback on selected relays",
                "fresh shared-host and guest resource admission"],
            "limitations": ["synthetic workload, not production profile or tagged acceptance",
                "50 tracked contacts with three devices, three groups, plus 137 cache-only owners with one device; generated workload, not an exact production-account replay",
                "normal production TLS, WebRTC, STUN and FIPS settings; no test trust roots",
                "no real human messages or copied live ratchet state"]
        });
        private_json(&inputs.output.join("receipt.json"), &receipt)?;
        Ok(receipt)
    }

    #[test]
    #[ignore = "explicit private output and bounded generation admission required"]
    fn export_populated_native_vm_profile() {
        let path = std::env::var_os("IRIS_SYNTHETIC_VM_INPUTS").expect("explicit fixture inputs");
        let inputs: Inputs = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        inputs.validate().unwrap();
        let epoch = unix_now().get() - 10;
        let (mut fixture, mut events) =
            synthetic_history_fixture_with_devices_at(100, 50, 3, 3, epoch);
        let cache_only = add_cache_only_rosters(&mut fixture.core, &mut events, 137, epoch);
        let result = export(&inputs, fixture, events, &cache_only).unwrap();
        println!("synthetic VM fixture exported: {}", result["counts"]);
    }

    fn add_cache_only_rosters(
        core: &mut AppCore,
        events: &mut Vec<Event>,
        count: usize,
        epoch: u64,
    ) -> Vec<(PublicKey, PublicKey)> {
        (0..count)
            .map(|_| {
                let owner = Keys::generate();
                let device = Keys::generate();
                let at = epoch + 5;
                let roster = AppKeys::new(vec![DeviceEntry::new(device.public_key(), at)]);
                let event = synthetic_discovery_event(
                    roster.get_event_at(owner.public_key(), at),
                    &owner,
                    epoch,
                );
                core.protocol_engine
                    .as_mut()
                    .unwrap()
                    .ingest_app_keys_event(&event)
                    .unwrap();
                core.capture_authorization_event(&event).unwrap();
                core.app_keys.insert(
                    owner.public_key().to_hex(),
                    account_app_keys::known_app_keys_from_ndr(owner.public_key(), &roster, at),
                );
                events.push(event);
                (owner.public_key(), device.public_key())
            })
            .collect()
    }

    fn inputs(parent: &Path) -> Inputs {
        Inputs {
            output: parent.join("export"),
            run_id: "synthetic-vm-export-test".into(),
            relay_urls: (0..5).map(|i| format!("wss://relay-{i}.invalid")).collect(),
        }
    }

    #[test]
    fn synthetic_vm_seed_corpus_rejects_unsafe_unbounded_or_expired_events() {
        let keys = Keys::generate();
        let device = Keys::generate();
        let now = unix_now().get();
        let valid = synthetic_discovery_event(
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)])
                .get_event_at(keys.public_key(), now),
            &keys,
            now - 10,
        );
        seed_corpus(vec![valid.clone()]).unwrap();
        let mut tampered = valid.clone();
        tampered.content.push('x');
        assert!(seed_corpus(vec![tampered]).is_err());
        assert!(seed_corpus(vec![valid.clone(); 291]).is_err());
        let forbidden = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "synthetic")
            .tag(nostr::Tag::expiration(Timestamp::from_secs(now + 3600)))
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            seed_corpus(vec![forbidden]).is_err(),
            "messages and group ciphertext cannot be seeded"
        );
        for expiry in [None, Some(now - 1), Some(now + 2 * 86400)] {
            let mut unsigned = AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)])
                .get_event_at(keys.public_key(), now);
            if let Some(expiry) = expiry {
                unsigned
                    .tags
                    .push(nostr::Tag::expiration(Timestamp::from_secs(expiry)));
                unsigned.id = None;
            }
            assert!(seed_corpus(vec![unsigned.sign_with_keys(&keys).unwrap()]).is_err());
        }
        let large = EventBuilder::new(
            Kind::from(INVITE_EVENT_KIND as u16),
            "x".repeat(1024 * 1024),
        )
        .tag(nostr::Tag::expiration(Timestamp::from_secs(now + 3600)))
        .sign_with_keys(&keys)
        .unwrap();
        assert!(seed_corpus(vec![large]).is_err());
    }

    #[test]
    fn synthetic_vm_export_rejects_unsafe_or_ambiguous_inputs() {
        let parent = tempfile::tempdir().unwrap();
        let mut input = inputs(&parent.path().canonicalize().unwrap());
        input.validate().unwrap();
        input.relay_urls[0] = "ws://relay-0.invalid".into();
        assert!(input.validate().is_err());
        input.relay_urls[0] = input.relay_urls[1].clone();
        assert!(input.validate().is_err());
        input.relay_urls[0] = "wss://relay-0.invalid".into();
        input.run_id = "ordinary-profile".into();
        assert!(input.validate().is_err());
        input.run_id = "synthetic-vm-export-test".into();
        std::fs::create_dir(&input.output).unwrap();
        assert!(
            input.validate().is_err(),
            "existing outputs must remain untouched"
        );
        let profile = parent.path().join("inventory");
        std::fs::create_dir_all(profile.join("attachments/blobs")).unwrap();
        std::fs::write(profile.join("core.sqlite3"), b"synthetic").unwrap();
        std::fs::write(profile.join("attachments/blobs/data.mdb"), b"synthetic").unwrap();
        let (files, directories) = inventory_profile(&profile).unwrap();
        assert!(files.contains_key("profile/attachments/blobs/data.mdb"));
        assert!(directories.contains(&"profile/attachments/blobs".to_owned()));
        std::fs::create_dir(profile.join("unexpected")).unwrap();
        assert!(inventory_profile(&profile)
            .unwrap_err()
            .to_string()
            .contains("unexpected"));
        std::fs::remove_dir(profile.join("unexpected")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            assert_eq!(
                std::fs::metadata(profile.join("attachments"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(profile.join("attachments/blobs/data.mdb"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            symlink(parent.path(), profile.join("attachments/escape")).unwrap();
            assert!(inventory_profile(&profile)
                .unwrap_err()
                .to_string()
                .contains("escape"));
            std::fs::remove_file(profile.join("attachments/escape")).unwrap();
            let _socket =
                std::os::unix::net::UnixListener::bind(profile.join("attachments/socket")).unwrap();
            assert!(inventory_profile(&profile)
                .unwrap_err()
                .to_string()
                .contains("socket"));
        }
    }

    #[test]
    fn synthetic_vm_export_reopens_real_history_and_swift_account_bundle() {
        let parent = tempfile::tempdir().unwrap();
        let input = inputs(&parent.path().canonicalize().unwrap());
        let epoch = unix_now().get() - 10;
        let (mut fixture, mut events) =
            synthetic_history_fixture_with_devices_at(2, 4, 2, 3, epoch);
        let cache_only = add_cache_only_rosters(&mut fixture.core, &mut events, 7, epoch);
        assert!(events
            .iter()
            .all(|event| event.created_at.as_secs() >= epoch));
        let before = fixture
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot();
        let event_count = events
            .iter()
            .filter(|event| event.kind != Kind::Metadata)
            .count();
        let receipt = export(&input, fixture, events, &cache_only).unwrap();
        assert_eq!(receipt["counts"]["threads"], 6);
        assert_eq!(receipt["counts"]["groups"], 2);
        assert_eq!(receipt["counts"]["appKeys"], 12);
        assert_eq!(receipt["counts"]["cacheOnlyOwners"], 7);
        assert_eq!(receipt["counts"]["cacheOnlyDiscoveryAuthors"], 0);
        assert_eq!(receipt["counts"]["relayEvents"], event_count);
        assert_eq!(receipt["launchAuthorized"], false);
        assert!(receipt["files"]
            .get("profile/attachments/index.sqlite3")
            .is_some());
        assert!(receipt["files"]
            .get("profile/attachments/blobs/data.mdb")
            .is_some());
        assert!(receipt["directories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "profile/attachments/blobs"));
        let profile = input.output.join("profile");
        let secret: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile.join("account-secret.json")).unwrap())
                .unwrap();
        let owner = Keys::parse(secret["ownerNsec"].as_str().unwrap()).unwrap();
        let device = Keys::parse(secret["deviceNsec"].as_str().unwrap()).unwrap();
        assert_eq!(
            owner.public_key().to_hex(),
            secret["ownerPubkeyHex"].as_str().unwrap()
        );
        let mut restored = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            profile.to_string_lossy().into_owned(),
        );
        let persisted = restored.load_persisted().unwrap().unwrap();
        assert_eq!(persisted.threads.len(), 6);
        assert_eq!(persisted.groups.len(), 2);
        assert_eq!(persisted.app_keys.len(), 12);
        assert_eq!(persisted.preferences.nostr_relay_urls, input.relay_urls);
        let defaults = PreferencesSnapshot::default();
        assert!(defaults.nearby_enabled);
        assert_eq!(
            persisted.preferences.nearby_enabled,
            defaults.nearby_enabled
        );
        assert_eq!(
            persisted.preferences.nearby_bluetooth_enabled,
            defaults.nearby_bluetooth_enabled
        );
        assert_eq!(
            persisted.preferences.nearby_lan_enabled,
            defaults.nearby_lan_enabled
        );
        assert_eq!(
            receipt["transportPreferences"]["nearbyEnabled"],
            persisted.preferences.nearby_enabled
        );
        assert_eq!(
            receipt["transportPreferences"]["nearbyBluetoothEnabled"],
            persisted.preferences.nearby_bluetooth_enabled
        );
        assert_eq!(
            receipt["transportPreferences"]["nearbyLanEnabled"],
            persisted.preferences.nearby_lan_enabled
        );
        assert!(
            restored
                .protocol_engine
                .as_ref()
                .unwrap()
                .session_manager_snapshot()
                == before,
            "export must preserve the generated ratchet checkpoint exactly"
        );
        let connection = restored.shared_db();
        let indexed: bool = connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages_fts WHERE messages_fts MATCH 'quiet')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(indexed, "export must retain the real full-text index");
        restored.shutdown();
        let serialized_receipt =
            std::fs::read_to_string(input.output.join("receipt.json")).unwrap();
        assert!(
            !serialized_receipt.contains("nsec1"),
            "receipt must not contain generated secret keys"
        );
    }
}
