async fn nearby_privacy_endpoint(
    keys: &Keys,
    peers: Vec<fips_core::config::PeerConfig>,
) -> Arc<fips_core::FipsEndpoint> {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.transports.udp =
        fips_core::config::TransportInstances::Single(fips_core::config::UdpConfig {
            bind_addr: Some("127.0.0.1:0".into()),
            advertise_on_nostr: Some(false),
            accept_connections: Some(true),
            ..Default::default()
        });
    config.peers = peers;
    Arc::new(
        fips_core::FipsEndpoint::builder()
            .config(config)
            .identity_nsec(keys.secret_key().to_secret_hex())
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    )
}

#[test]
fn private_provider_adapter_uses_unrelated_sources_for_copies_and_retry() {
    use crate::core::mesh_pubsub::PrivateMeshProviders;
    use fips_core::config::PeerConfig;
    use nostr_pubsub::VerifiedEvent;
    use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let provider_keys = Keys::generate();
        let provider_endpoint = nearby_privacy_endpoint(&provider_keys, Vec::new()).await;
        let provider = FipsPubsubClient::start(
            provider_endpoint.clone(),
            FipsPubsubClientOptions {
                accept_anonymous_publications: true,
                max_inbound_routed_peers: 8,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let address = provider_endpoint.bound_udp_listen_addrs().await.unwrap()[0];
        let adapter = PrivateMeshProviders::new(
            vec![PeerConfig::new(
                provider_endpoint.npub(),
                "udp",
                address.to_string(),
            )],
            None,
            vec![provider_endpoint.npub().to_string()],
            Vec::new(),
        );
        let own_device = Keys::generate();
        let peer_device = Keys::generate();
        let event = |recipient: &Keys, content: &str| {
            VerifiedEvent::try_from(
                EventBuilder::new(Kind::Custom(1060), content)
                    .tags([
                        nostr::Tag::parse(["v", "2"]).unwrap(),
                        nostr::Tag::public_key(recipient.public_key()),
                    ])
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
            )
            .unwrap()
        };
        let own_copy = event(&own_device, "opaque own-device copy");
        let peer_copy = event(&peer_device, "opaque peer copy");
        let mut sources = HashSet::new();
        for copy in [own_copy, peer_copy.clone(), peer_copy] {
            adapter
                .publish(copy, Arc::new(std::sync::atomic::AtomicBool::new(false)))
                .await
                .unwrap();
            let peers = provider_endpoint.peers().await.unwrap();
            for peer in peers {
                sources.insert(peer.npub);
            }
        }
        assert_eq!(
            sources.len(),
            3,
            "each copy and retry needs an unrelated authenticated source"
        );
        for key in [&own_device, &peer_device] {
            assert!(!sources.contains(&key.public_key().to_bech32().unwrap()));
        }
        assert_eq!(
            provider.delivery_snapshot().event_frames_received,
            3,
            "all three authenticated sources must actually send a serialized publication"
        );
        provider.shutdown_shared().await;
        provider_endpoint.shutdown().await.unwrap();
    });
}

#[test]
fn nearby_private_transcript_targets_only_the_intended_live_device_after_restart() {
    use crate::core::fips_nearby::{
        encode_fips_nearby_event, FipsNearbyOutbox, FipsNearbyPacket, FIPS_NEARBY_PORT,
    };
    use fips_core::config::PeerConfig;
    use std::collections::BTreeSet;

    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let stranger_owner = Keys::generate();
    let stranger_device = Keys::generate();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("nearby-private-transcript", &owner, &device);
    let source = core
        .runtime
        .block_on(nearby_privacy_endpoint(&device, vec![]));
    let source_addr = core
        .runtime
        .block_on(source.bound_udp_listen_addrs())
        .unwrap()[0]
        .to_string();
    let peer = core.runtime.block_on(nearby_privacy_endpoint(
        &peer_device,
        vec![PeerConfig::new(source.npub(), "udp", &source_addr)],
    ));
    let stranger = core.runtime.block_on(nearby_privacy_endpoint(
        &stranger_device,
        vec![PeerConfig::new(source.npub(), "udp", &source_addr)],
    ));
    let peer_receiver = core
        .runtime
        .block_on(peer.register_service_receiver(FIPS_NEARBY_PORT))
        .unwrap();
    let stranger_receiver = core
        .runtime
        .block_on(stranger.register_service_receiver(FIPS_NEARBY_PORT))
        .unwrap();
    core.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let peers = source.peers().await.unwrap();
                if [peer.npub(), stranger.npub()]
                    .iter()
                    .all(|npub| peers.iter().any(|p| p.connected && p.npub == *npub))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    });
    let (tcp, _) = super::device_sync_tcp::DeviceSyncTcpSender::test_channel(1, 1024);
    core.install_device_sync_sender_for_test(source.clone(), tcp, vec![]);
    core.preferences.nearby_enabled = true;
    core.preferences.nearby_mailbag_enabled = true;
    core.device_sync.as_mut().unwrap().nearby_enabled = true;
    core.app_keys.insert(
        stranger_owner.public_key().to_hex(),
        KnownAppKeys {
            owner_pubkey_hex: stranger_owner.public_key().to_hex(),
            created_at_secs: unix_now().get(),
            devices: vec![KnownAppKeyDevice {
                identity_pubkey_hex: stranger_device.public_key().to_hex(),
                created_at_secs: unix_now().get(),
                device_label: None,
                client_label: None,
                label_updated_at_secs: 0,
            }],
        },
    );

    let mut recipient = test_protocol_engine(&peer_owner, &peer_device);
    observe_current_device_appkeys_for_test(&mut recipient, &peer_owner, &peer_device);
    observe_peer_appkeys_for_test(
        &mut recipient,
        &owner,
        &[device.public_key()],
        unix_now().get(),
    );
    let recipient_invite = recipient.local_invite().unwrap();
    let engine = core.protocol_engine.as_mut().unwrap();
    engine.authenticate_local_owner_for_sending(&owner).unwrap();
    observe_current_device_appkeys_for_test(engine, &owner, &device);
    observe_local_invite_for_test(engine, &peer_owner, &peer_device, &recipient_invite);
    let sent = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "private first contact",
            None,
            UnixSeconds(unix_now().get()),
        )
        .unwrap();
    let expected = protocol_publish_events(&sent.effects)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert!(expected
        .iter()
        .any(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND));
    assert!(expected
        .iter()
        .any(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND));
    for effect in sent.effects {
        let ProtocolEffect::Publish(publish) = effect;
        assert!(core.publish_protocol_event(publish));
    }
    let received = core.runtime.block_on(async {
        let mut received = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while received.len() < expected.len() {
                let mut batch = Vec::new();
                peer_receiver.recv_batch_into(&mut batch, 16).await.unwrap();
                for packet in batch {
                    assert_eq!(packet.source_peer.npub(), source.npub());
                    let FipsNearbyPacket::Event { event_json, .. } =
                        FipsNearbyPacket::decode(packet.data.as_slice()).unwrap()
                    else {
                        panic!("unexpected receipt")
                    };
                    received.push(serde_json::from_str::<Event>(&event_json).unwrap());
                }
            }
        })
        .await
        .unwrap();
        let mut batch = Vec::new();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(200),
                stranger_receiver.recv_batch_into(&mut batch, 16)
            )
            .await
            .is_err(),
            "an unrelated connected contact must receive no private first-contact event"
        );
        received
    });
    assert_eq!(
        received
            .iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>(),
        expected
            .iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>()
    );
    for event in &received {
        let wire = serde_json::to_string(event).unwrap();
        assert!(!wire.contains(&owner.public_key().to_hex()));
        assert!(!wire.contains(&device.public_key().to_hex()));
        assert!(!wire.contains(&stranger_device.public_key().to_hex()));
        let targets = core.private_nearby_targets(event).unwrap();
        assert_eq!(targets, BTreeSet::from([peer.npub().to_owned()]));
    }
    let mut plaintexts = Vec::new();
    for event in received
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
    {
        recipient.observe_invite_response_event(event).unwrap();
    }
    for event in received
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
    {
        if let Some(message) = recipient.process_direct_message_event(event).unwrap() {
            plaintexts.push(message.content);
        }
    }
    assert!(plaintexts
        .iter()
        .any(|text| text.contains("private first contact")));

    let queue = core.device_sync.as_ref().unwrap().nearby_outbox.clone();
    assert_eq!(
        queue.read().unwrap().pending_for_link(peer.npub(), 2).len(),
        expected.len()
    );
    assert!(queue
        .read()
        .unwrap()
        .pending_for_link(stranger.npub(), 2)
        .is_empty());
    assert!(!queue
        .write()
        .unwrap()
        .acknowledge(stranger.npub(), &expected[0].id.to_hex()));
    let saved = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    assert_eq!(saved.len(), expected.len());
    let encrypted = expected
        .iter()
        .find(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap();
    let wake = EventBuilder::new(
        Kind::Custom(21_111),
        serde_json::json!({"v":3,"type":"call-wake","events":[encrypted]}).to_string(),
    )
    .tag(nostr::Tag::public_key(peer_device.public_key()))
    .sign_with_keys(&Keys::generate())
    .unwrap();
    core.publish_fips_nearby(&wake);
    core.runtime.block_on(async {
        let mut batch = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), peer_receiver.recv_batch_into(&mut batch, 16)).await.unwrap().unwrap();
        assert!(batch.iter().any(|packet| matches!(FipsNearbyPacket::decode(packet.data.as_slice()), Some(FipsNearbyPacket::Event { event_id, .. }) if event_id == wake.id.to_hex())));
        batch.clear();
        assert!(tokio::time::timeout(Duration::from_millis(200), stranger_receiver.recv_batch_into(&mut batch, 16)).await.is_err(),
            "call wakeups must not become unrelated-neighbor mailbags");
    });
    let group = core
        .protocol_engine
        .as_mut()
        .unwrap()
        .create_group(
            "private distribution".into(),
            vec![peer_owner.public_key()],
            unix_now(),
        )
        .unwrap();
    let group_id = group.snapshot.as_ref().unwrap().group_id.clone();
    let group_events = protocol_publish_events(&group.effects)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert!(!group_events.is_empty());
    for effect in group.effects {
        let ProtocolEffect::Publish(publish) = effect;
        assert!(core.publish_protocol_event(publish));
    }
    let group_received = core.runtime.block_on(async {
        let mut events = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while events.len() < group_events.len() {
                let mut packets = Vec::new();
                peer_receiver
                    .recv_batch_into(&mut packets, 16)
                    .await
                    .unwrap();
                for packet in packets {
                    if let Some(FipsNearbyPacket::Event { event_json, .. }) =
                        FipsNearbyPacket::decode(packet.data.as_slice())
                    {
                        events.push(serde_json::from_str::<Event>(&event_json).unwrap());
                    }
                }
            }
        })
        .await
        .unwrap();
        let mut packets = Vec::new();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(200),
                stranger_receiver.recv_batch_into(&mut packets, 16)
            )
            .await
            .is_err(),
            "group metadata and sender keys must reach only their intended participant"
        );
        events
    });
    let mut group_plaintexts = Vec::new();
    for event in group_received {
        let wire = serde_json::to_string(&event).unwrap();
        assert!(!wire.contains(&group_id));
        assert!(!wire.contains(&owner.public_key().to_hex()));
        assert_eq!(
            event.tags.public_keys().copied().collect::<Vec<_>>(),
            vec![peer_device.public_key()]
        );
        if let Some(message) = recipient.process_direct_message_event(&event).unwrap() {
            group_plaintexts.push(message.content);
        }
    }
    assert!(
        group_plaintexts
            .iter()
            .any(|content| content.contains("10446")),
        "the actual sender-key distribution must traverse the private wire"
    );
    core.stop_device_sync_now();
    core.runtime.block_on(async {
        peer.shutdown().await.unwrap();
        stranger.shutdown().await.unwrap();
    });
    drop(core);
    let restored = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        directory.path().to_string_lossy().into_owned(),
    );
    let mut restored_queue = FipsNearbyOutbox::default();
    for pending in saved {
        let event = serde_json::from_str::<Event>(&pending.event_json).unwrap();
        restored_queue.insert_for_targets(
            event.id.to_hex(),
            encode_fips_nearby_event(&event).unwrap(),
            restored.private_nearby_targets(&event),
        );
    }
    assert_eq!(
        restored_queue.pending_for_link(peer.npub(), 3).len(),
        expected.len()
    );
    assert!(restored_queue
        .pending_for_link(stranger.npub(), 3)
        .is_empty());
}

#[test]
fn nearby_private_unknown_invite_mapping_never_selects_a_neighbor() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (core, _, _directory) =
        logged_in_test_core_with_updates("nearby-unknown-invite", &owner, &device);
    let event = EventBuilder::new(
        Kind::Custom(INVITE_RESPONSE_KIND as u16),
        "opaque handshake",
    )
    .tag(nostr::Tag::parse(["v", "2"]).unwrap())
    .tag(nostr::Tag::public_key(Keys::generate().public_key()))
    .sign_with_keys(&Keys::generate())
    .unwrap();
    assert!(core.private_nearby_targets(&event).unwrap().is_empty());
}

#[test]
fn nearby_private_invite_response_accepts_only_current_local_invite_keys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let mut receiver = logged_in_test_core("nearby-private-invite-receive", &owner, &device);
    let url = create_private_invite_for_test(&mut receiver);
    let invite = receiver
        .private_chat_invites
        .values()
        .next()
        .unwrap()
        .clone();
    let invite_key = super::invites::private_chat_invite_key(&invite);
    let response = crafted_private_invite_response(
        &url,
        &sender_device,
        Some(sender_device.public_key().to_hex()),
        Some(sender_owner.public_key()),
    );
    let wire = response.as_json();
    for key in [&owner, &device, &sender_owner, &sender_device] {
        assert!(!wire.contains(&key.public_key().to_hex()));
    }
    assert_ne!(
        response.tags.public_keys().next().copied(),
        receiver
            .protocol_engine
            .as_ref()
            .unwrap()
            .local_invite_response_pubkey()
    );
    let encoded = fips_nearby::encode_fips_nearby_event(&response).unwrap();
    let fips_nearby::FipsNearbyPacket::Event { event_json, .. } =
        fips_nearby::FipsNearbyPacket::decode(&encoded).unwrap()
    else {
        panic!("event packet")
    };
    let decoded: Event = serde_json::from_str(&event_json).unwrap();
    assert!(
        receiver.accepts_private_nearby_event(&decoded),
        "an authenticated locally issued private invite must be eligible for DR decryption"
    );
    let proof = AppKeys::new(vec![DeviceEntry::new(
        sender_device.public_key(),
        unix_now().get(),
    )])
    .get_event_at(sender_owner.public_key(), unix_now().get())
    .sign_with_keys(&sender_owner)
    .unwrap();
    ingest_invite_owner_app_keys(&mut receiver, sender_owner.public_key(), vec![proof]);
    receiver.handle_fips_nearby_packet(
        &sender_device.public_key().to_hex(),
        fips_nearby::FIPS_NEARBY_PORT,
        &encoded,
    );
    assert_eq!(
        stored_session_count(&receiver, sender_owner.public_key()),
        1,
        "the actual private handshake must authenticate and install its session"
    );
    assert!(!receiver.private_chat_invites.contains_key(&invite_key));
    assert!(
        !receiver.accepts_private_nearby_event(&decoded),
        "consumed invites must not remain receive targets"
    );
    let mut stale = invite.clone();
    stale.metadata_privacy_version = 1;
    receiver
        .private_chat_invites
        .insert(invite_key.clone(), stale);
    assert!(
        !receiver.accepts_private_nearby_event(&decoded),
        "old invite formats must not be restored as a fallback"
    );
    let mut foreign = invite;
    foreign.inviter_device_pubkey =
        NdrDevicePubkey::from_bytes(sender_device.public_key().to_bytes());
    receiver
        .private_chat_invites
        .insert(invite_key.clone(), foreign);
    assert!(
        !receiver.accepts_private_nearby_event(&decoded),
        "another device's invite must not be accepted locally"
    );
    receiver.private_chat_invites.remove(&invite_key);
    for recipient in [
        owner.public_key(),
        device.public_key(),
        Keys::generate().public_key(),
    ] {
        let other = EventBuilder::new(
            Kind::Custom(INVITE_RESPONSE_KIND as u16),
            "opaque handshake",
        )
        .tags([
            nostr::Tag::parse(["v", "2"]).unwrap(),
            nostr::Tag::public_key(recipient),
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();
        assert!(
            !receiver.accepts_private_nearby_event(&other),
            "identity and unrelated keys are not invite targets"
        );
    }
}

#[test]
fn native_identified_receiver_does_not_export_peer_public_cache_or_restart_replay() {
    use fips_core::config::PeerConfig;
    use nostr_pubsub::{EventBus, EventSource, InMemoryEventBus, VerifiedEvent};
    use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};

    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let directory = tempfile::tempdir().unwrap();
    let foreign = [0, 37195, 54321].map(|kind| {
        VerifiedEvent::try_from(EventBuilder::new(Kind::Custom(kind),
            r#"{"identifier":"fips-overlay-v1","version":1,"endpoints":[{"transport":"tcp","addr":"8.8.8.8:2121"}]}"#)
            .tag(nostr::Tag::identifier("fips-overlay-v1"))
            .sign_with_keys(&peer).unwrap()).unwrap()
    });
    let retained = Arc::new(InMemoryEventBus::default());
    for restart in [false, true] {
        let mut core = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            directory.path().to_string_lossy().into_owned(),
        );
        let endpoint = core
            .runtime
            .block_on(nearby_privacy_endpoint(&device, Vec::new()));
        let address = core
            .runtime
            .block_on(endpoint.bound_udp_listen_addrs())
            .unwrap()[0];
        let upstream = core.runtime.block_on(nearby_privacy_endpoint(
            &Keys::generate(),
            vec![PeerConfig::new(endpoint.npub(), "udp", address.to_string())],
        ));
        let stranger = core.runtime.block_on(nearby_privacy_endpoint(
            &Keys::generate(),
            vec![PeerConfig::new(endpoint.npub(), "udp", address.to_string())],
        ));
        let receiver = Arc::new(
            core.runtime
                .block_on(FipsPubsubClient::start_receive_only_with_public_authors(
                    endpoint.clone(),
                    FipsPubsubClientOptions {
                        routed_peers: vec![upstream.npub().into(), stranger.npub().into()],
                        ..Default::default()
                    },
                    core.mesh_public_service_authors(),
                ))
                .unwrap(),
        );
        let (tcp, _) = super::device_sync_tcp::DeviceSyncTcpSender::test_channel(1, 1024);
        core.install_device_sync_sender_for_test(endpoint.clone(), tcp, Vec::new());
        core.device_sync.as_mut().unwrap().pubsub = Some(receiver.clone());
        core.reconcile_mesh_protocol_subscriptions();
        core.runtime.block_on(async {
            let upstream_client = FipsPubsubClient::start(
                upstream.clone(),
                FipsPubsubClientOptions {
                    routed_peers: vec![endpoint.npub().into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let stranger_client = FipsPubsubClient::start(
                stranger.clone(),
                FipsPubsubClientOptions {
                    routed_peers: vec![endpoint.npub().into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let global = receiver
                .subscribe(vec![Filter::new().kinds([
                    Kind::Metadata,
                    Kind::Custom(37195),
                    Kind::Custom(54321),
                ])])
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                while upstream_client.peer_subscription_count().unwrap() < 2
                    || receiver.delivery_snapshot().req_frames_received < 2
                {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("both actual public-provider TCP request paths must connect");
            if !restart {
                for event in &foreign {
                    upstream_client
                        .publish(event.clone(), EventSource::local_index("public-provider"))
                        .await
                        .unwrap();
                    retained
                        .publish(
                            event.clone(),
                            EventSource::local_index("private-lookup-retained"),
                        )
                        .await
                        .unwrap();
                }
                tokio::time::timeout(Duration::from_secs(10), async {
                    while receiver.delivery_snapshot().event_frames_received < foreign.len() as u64
                    {
                        sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("the identified receiver must actually cache the peer events");
            }
            // Stress the final network boundary even if a future host reader
            // accidentally offers privately learned public data for replay.
            receiver.set_replay_source(Some(retained.clone())).unwrap();
            let before = receiver.delivery_snapshot().req_frames_received;
            let mut incoming = stranger_client
                .subscribe(vec![
                    Filter::new().kinds([Kind::Metadata, Kind::Custom(37195), Kind::Custom(54321)]),
                    Filter::new().ids(foreign.iter().map(|event| event.as_event().id)),
                ])
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                while receiver.delivery_snapshot().req_frames_received <= before {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("serialized broad and exact-ID REQ must reach the native receiver");
            assert!(
                tokio::time::timeout(Duration::from_millis(250), incoming.recv())
                    .await
                    .is_err(),
                "peer profile/advert/unknown-kind cache escaped on restart={restart}"
            );
            let denied = stranger_client.delivery_snapshot();
            assert_eq!(
                denied.event_frames_received, 0,
                "serialized peer EVENT escaped on restart={restart}"
            );
            assert_eq!(
                denied.inv_frames_received, 0,
                "serialized peer INV escaped on restart={restart}"
            );
            let own = VerifiedEvent::try_from(
                EventBuilder::new(Kind::Custom(37195), "own public endpoint advert")
                    .tag(nostr::Tag::identifier("fips-overlay-v1"))
                    .sign_with_keys(&device)
                    .unwrap(),
            )
            .unwrap();
            receiver
                .publish(own.clone(), EventSource::local_index("own-public-service"))
                .await
                .unwrap();
            let delivered = tokio::time::timeout(Duration::from_secs(10), incoming.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                delivered.event.as_event().id,
                own.as_event().id,
                "the approved own public advert must still cross the same transport"
            );
            assert!(stranger_client.delivery_snapshot().event_frames_received > 0);
            assert!(stranger_client.delivery_snapshot().inv_frames_received > 0);
            global.close();
            stranger_client.shutdown().await;
            upstream_client.shutdown().await;
        });
        core.stop_device_sync_now();
        core.runtime.block_on(async {
            upstream.shutdown().await.unwrap();
            stranger.shutdown().await.unwrap();
        });
        drop(core);
    }
}
