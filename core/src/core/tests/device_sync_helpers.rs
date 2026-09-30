fn configure_test_device_sync_profile(
    core: &mut AppCore,
    owner: &Keys,
    local_device: &Keys,
    sibling_device: &Keys,
    relay_url: Option<&str>,
) {
    core.logged_in.as_mut().expect("logged in").relay_urls =
        relay_urls_from_strings(&relay_url.into_iter().map(str::to_string).collect::<Vec<_>>());
    let owner_hex = owner.public_key().to_hex();
    core.app_keys.insert(
        owner_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: owner_hex,
            created_at_secs: 100,
            devices: vec![
                KnownAppKeyDevice {
                    identity_pubkey_hex: local_device.public_key().to_hex(),
                    created_at_secs: 1,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
                KnownAppKeyDevice {
                    identity_pubkey_hex: sibling_device.public_key().to_hex(),
                    created_at_secs: 100,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
            ],
        },
    );
}

fn test_fips_peer(keys: &Keys) -> fips_core::PeerIdentity {
    fips_core::PeerIdentity::from_npub(
        &keys
            .public_key()
            .to_bech32()
            .expect("encode test device npub"),
    )
    .expect("test FIPS identity")
}

fn test_keys_with_compressed_prefix(prefix: u8) -> Keys {
    loop {
        let keys = Keys::generate();
        let identity = fips_core::Identity::from_secret_str(&keys.secret_key().to_secret_hex())
            .expect("test FIPS identity from Nostr secret");
        if identity.pubkey_full().serialize()[0] == prefix {
            return keys;
        }
    }
}

fn reserve_tcp_addr() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve TCP address");
    listener.local_addr().expect("reserved TCP address")
}

async fn device_sync_peer_transport(
    endpoint: &fips_core::FipsEndpoint,
    peer: &fips_core::PeerIdentity,
) -> Option<String> {
    endpoint
        .peers()
        .await
        .expect("device-sync peer health")
        .into_iter()
        .find(|candidate| candidate.connected && candidate.npub == peer.npub())
        .and_then(|candidate| candidate.transport_type)
}

async fn device_sync_pair_is_connected(
    link: [(&fips_core::FipsEndpoint, &fips_core::PeerIdentity); 2],
) -> bool {
    device_sync_peer_transport(link[0].0, link[0].1)
        .await
        .is_some()
        && device_sync_peer_transport(link[1].0, link[1].1)
            .await
            .is_some()
}

async fn device_sync_pair_uses_websocket(
    link: [(&fips_core::FipsEndpoint, &fips_core::PeerIdentity); 2],
) -> bool {
    device_sync_peer_transport(link[0].0, link[0].1)
        .await
        .as_deref()
        == Some("websocket")
        && device_sync_peer_transport(link[1].0, link[1].1)
            .await
            .as_deref()
            == Some("websocket")
}

fn has_device_sync_message(core: &AppCore, chat_id: &str, message_id: &str) -> bool {
    core.threads.get(chat_id).is_some_and(|thread| {
        thread
            .messages
            .iter()
            .any(|message| message.id == message_id)
    })
}

fn wait_for_device_sync_message(
    (sender, sender_messages): (&mut AppCore, &flume::Receiver<CoreMsg>),
    (receiver, messages): (&mut AppCore, &flume::Receiver<CoreMsg>),
    link: [(&fips_core::FipsEndpoint, &fips_core::PeerIdentity); 2],
    chat_id: &str,
    message_id: &str,
    stable_for: Duration,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10) + stable_for;
    let mut first_seen = None;
    loop {
        while let Ok(message) = sender_messages.try_recv() {
            sender.handle_message(message);
        }
        while let Ok(message) = messages.try_recv() {
            receiver.handle_message(message);
        }
        let now = std::time::Instant::now();
        if has_device_sync_message(receiver, chat_id, message_id) {
            first_seen.get_or_insert(now);
        }
        assert!(
            sender.runtime.block_on(device_sync_pair_is_connected(link)),
            "production device sync must retain an authenticated FIPS route"
        );
        if first_seen.is_some_and(|seen| now.duration_since(seen) >= stable_for) {
            return;
        }
        assert!(
            now < deadline,
            "production fips-tcp device sync should converge: message={message_id}",
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
