use super::*;

fn offer(keys: &Keys) -> Offer {
    Offer {
        id: "ab".repeat(16),
        token: "cd".repeat(32),
        owner: "01".repeat(32),
        recipient: "02".repeat(32),
        device: keys.public_key().to_hex(),
        caption: "For later".into(),
        expires_at_secs: unix_now().get() + 86_400,
        files: vec![ManifestFile {
            filename: "notes.txt".into(),
            size_bytes: 4,
            sha256: "ef".repeat(32),
        }],
    }
}

#[test]
fn signed_direct_offer_cannot_be_tampered_with_or_claim_another_device() {
    let keys = Keys::generate();
    let original = offer(&keys);
    let wire = original.wire(&keys).unwrap();
    assert_eq!(parse(&wire).unwrap().device, keys.public_key().to_hex());
    assert!(parse(&wire.replace("notes.txt", "other.txt")).is_none());
    let mut forged = original;
    forged.device = Keys::generate().public_key().to_hex();
    assert!(parse(&forged.wire(&keys).unwrap()).is_none());
    assert_eq!(preview(&wire).as_deref(), Some("For later"));
    assert!(!preview(&wire).unwrap().contains(&"cd".repeat(32)));
}

#[test]
fn only_signed_unexpired_self_offers_can_sync_before_publication() {
    let keys = Keys::generate();
    let mut offer = offer(&keys);
    offer.recipient = offer.owner.clone();
    let wire = offer.wire(&keys).unwrap();
    assert!(is_pending_self_offer(
        &wire,
        &offer.owner,
        Some(&offer.owner),
        true
    ));
    assert!(!is_pending_self_offer(
        &wire,
        &offer.owner,
        Some(&offer.owner),
        false
    ));
    assert!(!is_pending_self_offer(
        &wire,
        &offer.owner,
        Some(&"03".repeat(32)),
        true
    ));
    assert!(!is_pending_self_offer(
        "ordinary draft",
        &offer.owner,
        Some(&offer.owner),
        true
    ));
    assert!(!is_pending_self_offer(
        &wire.replace("notes.txt", "other.txt"),
        &offer.owner,
        Some(&offer.owner),
        true
    ));
    offer.recipient = "02".repeat(32);
    assert!(!is_pending_self_offer(
        &offer.wire(&keys).unwrap(),
        &offer.owner,
        Some(&offer.owner),
        true
    ));
}

#[test]
fn direct_offer_rejects_unsafe_names_and_unbounded_manifests() {
    let keys = Keys::generate();
    let mut invalid = offer(&keys);
    for filename in ["../secret", "a/b", "a\\b", "file:stream", "..", "bad\nname"] {
        invalid.files[0].filename = filename.into();
        assert!(
            parse(&invalid.wire(&keys).unwrap()).is_none(),
            "{filename:?}"
        );
    }
    invalid.files[0].filename = "notes.txt".into();
    invalid.files[0].size_bytes = MAX_FILE_BYTES + 1;
    assert!(parse(&invalid.wire(&keys).unwrap()).is_none());
    invalid.files = vec![offer(&keys).files[0].clone(); MAX_FILES + 1];
    assert!(parse(&invalid.wire(&keys).unwrap()).is_none());
}

#[test]
fn preparing_direct_files_owns_stable_copies_and_cleans_up_failed_batches() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("chosen.txt");
    std::fs::write(&source, b"before").unwrap();
    let keys = Keys::generate();
    let mut manifest = offer(&keys);
    manifest.files.clear();
    let record = Record {
        chat_id: manifest.recipient.clone(),
        wire: String::new(),
        offer: manifest,
        is_sender: true,
        status: DirectFileTransferStatus::Offered,
        paths: vec![],
        peer: None,
        transferred: 0,
        error: None,
    };
    let selected = OutgoingAttachment {
        file_path: source.to_string_lossy().into_owned(),
        filename: "chosen.txt".into(),
    };
    let prepared = prepare::prepare(
        dir.path().join("send"),
        vec![selected.clone()],
        record.clone(),
        keys.clone(),
    )
    .unwrap();
    std::fs::write(&source, b"after").unwrap();
    assert_eq!(std::fs::read(&prepared.paths[0]).unwrap(), b"before");
    assert!(prepared.paths[0].ends_with("0-chosen.txt"));
    assert_eq!(parse(&prepared.wire).unwrap().files[0].size_bytes, 6);
    let bad = OutgoingAttachment {
        filename: "../bad".into(),
        ..selected.clone()
    };
    let destination = dir.path().join("failed");
    assert!(prepare::prepare(destination.clone(), vec![selected, bad], record, keys).is_err());
    assert!(!destination.exists());
}

#[test]
fn interrupted_transfers_never_become_replayable_on_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::core::storage::open_database(dir.path()).unwrap();
    let keys = Keys::generate();
    let offer = offer(&keys);
    let record = Record {
        chat_id: offer.recipient.clone(),
        wire: offer.wire(&keys).unwrap(),
        offer,
        is_sender: true,
        status: DirectFileTransferStatus::Transferring,
        paths: vec![],
        peer: Some("01".repeat(32)),
        transferred: 3,
        error: None,
    };
    storage::save(&db, &record).unwrap();
    storage::interrupt(&db).unwrap();
    assert_eq!(
        storage::load(&db, &record.offer.id)
            .unwrap()
            .unwrap()
            .status,
        DirectFileTransferStatus::Unavailable
    );
}

#[test]
fn direct_file_storage_upgrades_the_existing_contact_memory_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::core::storage::open_database(dir.path()).unwrap();
    db.lock()
        .unwrap()
        .execute_batch(
            "DROP TABLE direct_file_transfers;
         INSERT INTO app_meta(key,value) VALUES('direct-files-migration-test','keep');
         PRAGMA user_version=38;",
        )
        .unwrap();
    drop(db);
    let upgraded = crate::core::storage::open_database(dir.path()).unwrap();
    assert!(storage::all(&upgraded).unwrap().is_empty());
    let value: String = upgraded
        .lock()
        .unwrap()
        .query_row(
            "SELECT value FROM app_meta WHERE key='direct-files-migration-test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(value, "keep");
}
