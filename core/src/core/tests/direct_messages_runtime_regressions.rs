#[test]
fn incoming_device_authored_runtime_message_from_known_peer_routes_to_owner_thread() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("remote-device-authored-runtime", &owner, &local_device);
    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    core.apply_known_app_keys_snapshot(peer_owner.public_key(), &peer_app_keys, 1);
    let peer_owner_chat_id = peer_owner.public_key().to_hex();
    let peer_device_chat_id = peer_device.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        peer_device.public_key(),
        CHAT_MESSAGE_KIND,
        "sent as peer device",
        1_777_159_504,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message_with_metadata(
        peer_owner.public_key(),
        Some(peer_device.public_key()),
        None,
        content,
        Some("e".repeat(64)),
    );

    let thread = core
        .threads
        .get(&peer_owner_chat_id)
        .expect("peer owner thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].id, inner_id);
    assert_eq!(thread.messages[0].body, "sent as peer device");
    assert!(
        !core.threads.contains_key(&peer_device_chat_id),
        "authenticated device-authored rumors must still route to the owner thread"
    );
}

#[test]
fn direct_group_pairwise_payload_that_looks_like_runtime_rumor_is_applied() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let admin_owner = Keys::generate();
    let admin_device = Keys::generate();
    let mut core = logged_in_test_core("direct-runtime-shaped-group-pairwise", &owner, &device);
    let admin_app_keys = AppKeys::new(vec![DeviceEntry::new(admin_device.public_key(), 1)]);
    core.apply_known_app_keys_snapshot(admin_owner.public_key(), &admin_app_keys, 1);

    let group_id = "runtime-shaped-group-pairwise".to_string();
    let group = test_group_snapshot(
        &group_id,
        "Runtime Shaped",
        admin_owner.public_key(),
        vec![admin_owner.public_key(), owner.public_key()],
        vec![admin_owner.public_key()],
        1,
    );
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(admin_device.public_key()),
            created_at: nostr_double_ratchet::UnixSeconds(1_777_159_505),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot {
            snapshot: group.clone(),
        },
    )
    .expect("group metadata payload");
    let content = String::from_utf8(payload).expect("json group payload");
    assert!(
        looks_like_runtime_rumor(&content),
        "regression needs the group payload to share the runtime rumor shape"
    );

    core.apply_decrypted_runtime_message_with_metadata(
        admin_owner.public_key(),
        Some(admin_device.public_key()),
        None,
        content,
        Some("f".repeat(64)),
    );

    let applied = core.groups.get(&group_id).expect("group metadata applied");
    assert_eq!(applied.name, group.name);
    assert!(core.threads.contains_key(&group_chat_id(&group_id)));
}
