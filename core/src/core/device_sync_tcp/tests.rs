use super::*;

fn reserve_local_rendezvous_addr() -> std::net::SocketAddrV4 {
    let socket = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("reserve local rendezvous address");
    let std::net::SocketAddr::V4(address) =
        socket.local_addr().expect("reserved rendezvous address")
    else {
        panic!("IPv4 loopback bind returned IPv6");
    };
    address
}

async fn local_rendezvous_endpoint(address: std::net::SocketAddrV4) -> Arc<FipsEndpoint> {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.local.rendezvous_addr = address;
    config.node.discovery.local.retry_interval_ms = 20;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.routing.mode = fips_core::config::RoutingMode::ReplyLearned;
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .local_rendezvous()
            .without_system_tun()
            .bind()
            .await
            .expect("bind local FIPS endpoint"),
    )
}

async fn wait_for_authenticated_peer(endpoint: &FipsEndpoint, npub: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if endpoint
                .peers()
                .await
                .expect("peer snapshot")
                .iter()
                .any(|peer| peer.connected && peer.npub == npub)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("endpoint did not authenticate {npub}"));
}

#[test]
fn records_survive_split_and_coalesced_stream_reads() {
    let mut reader = RecordReader::new(8);
    let bytes = [frame(&[1, 2]), frame(&[3])].concat();
    assert!(reader.push(&bytes[..5]).unwrap().is_empty());
    assert_eq!(reader.push(&bytes[5..]).unwrap(), vec![vec![1, 2], vec![3]]);
    assert!(reader.push(&9_u32.to_be_bytes()).is_err());
}

#[test]
fn pending_records_are_bounded_and_resync_notice_has_priority() {
    let mut pending = HashMap::new();
    let mut count = 0;
    let peer = "peer".to_owned();
    for value in 0..MAX_PENDING_RECORDS_PER_PEER {
        assert!(enqueue_pending(
            &mut pending,
            &mut count,
            peer.clone(),
            frame(&[value as u8]),
            false,
            false,
        ));
    }
    assert!(!enqueue_pending(
        &mut pending,
        &mut count,
        peer.clone(),
        frame(b"dropped"),
        false,
        false,
    ));
    assert_eq!(pending[&peer].len(), MAX_PENDING_RECORDS_PER_PEER);
    assert_eq!(count, MAX_PENDING_RECORDS_PER_PEER);

    let request = frame(b"request");
    assert!(enqueue_pending(
        &mut pending,
        &mut count,
        peer.clone(),
        request.clone(),
        true,
        true,
    ));
    assert_eq!(pending[&peer].len(), MAX_PENDING_RECORDS_PER_PEER);
    assert_eq!(pending[&peer].front().unwrap().bytes, request);
    assert_eq!(count, MAX_PENDING_RECORDS_PER_PEER);
}

#[test]
fn priority_control_waits_behind_a_partially_written_frame() {
    let peer = "peer".to_owned();
    let original = frame(b"original");
    let control = frame(b"resync");
    let mut pending = HashMap::new();
    let mut count = 0;
    assert!(enqueue_pending(
        &mut pending,
        &mut count,
        peer.clone(),
        original.clone(),
        false,
        false,
    ));
    pending.get_mut(&peer).unwrap().front_mut().unwrap().offset = 3;
    assert!(enqueue_pending(
        &mut pending,
        &mut count,
        peer.clone(),
        control,
        true,
        true,
    ));

    let mut stream = original[..3].to_vec();
    for record in &pending[&peer] {
        stream.extend_from_slice(&record.bytes[record.offset..]);
    }
    assert_eq!(
        RecordReader::new(1024).push(&stream).unwrap(),
        vec![b"original".to_vec(), b"resync".to_vec()]
    );
}

#[test]
fn accepted_batch_over_pending_limit_is_deferred_without_loss() {
    let identity = fips_core::Identity::generate();
    let peer = PeerIdentity::from_pubkey_full(identity.pubkey_full());
    let records = (0..MAX_PENDING_RECORDS_PER_PEER + 7)
        .map(|value| vec![value as u8])
        .collect::<Vec<_>>();
    let batch = SendBatch {
        peer,
        records: records.clone().into(),
    };
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    let mut deferred = HashMap::new();
    let mut deferred_count = 0;
    defer_batch(batch, &dirty, &mut deferred, &mut deferred_count);
    let mut pending = HashMap::new();
    let mut count = 0;
    let mut queued = Vec::new();

    while deferred_count > 0 {
        enqueue_deferred_records(&mut deferred, &mut deferred_count, &mut pending, &mut count);
        let item = pending
            .get_mut(&peer.npub())
            .and_then(VecDeque::pop_front)
            .expect("a bounded batch chunk should be queued");
        count -= 1;
        queued.push(item.bytes);
    }
    if let Some(records) = pending.remove(&peer.npub()) {
        queued.extend(records.into_iter().map(|record| record.bytes));
    }
    assert_eq!(
        queued,
        records
            .iter()
            .map(|record| frame(record))
            .collect::<Vec<_>>()
    );
}

#[test]
fn offline_peer_batch_does_not_block_another_peers_record() {
    let first = PeerIdentity::from_pubkey_full(fips_core::Identity::generate().pubkey_full());
    let second = PeerIdentity::from_pubkey_full(fips_core::Identity::generate().pubkey_full());
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    let mut deferred = HashMap::new();
    let mut deferred_count = 0;
    defer_batch(
        SendBatch {
            peer: first,
            records: vec![b"offline".to_vec(); MAX_PENDING_RECORDS_PER_PEER + 1].into(),
        },
        &dirty,
        &mut deferred,
        &mut deferred_count,
    );
    defer_batch(
        SendBatch {
            peer: second,
            records: vec![b"connected".to_vec()].into(),
        },
        &dirty,
        &mut deferred,
        &mut deferred_count,
    );
    let mut pending = HashMap::new();
    let mut pending_count = 0;
    enqueue_deferred_records(
        &mut deferred,
        &mut deferred_count,
        &mut pending,
        &mut pending_count,
    );
    assert_eq!(pending[&first.npub()].len(), MAX_PENDING_RECORDS_PER_PEER);
    assert_eq!(pending[&second.npub()].len(), 1);
}

#[test]
fn four_full_offline_peer_queues_leave_capacity_for_a_fifth_peer() {
    let mut pending = HashMap::new();
    let mut pending_count = 0;
    for peer_index in 0..4 {
        for record_index in 0..MAX_PENDING_RECORDS_PER_PEER {
            assert!(enqueue_pending(
                &mut pending,
                &mut pending_count,
                format!("offline-{peer_index}"),
                frame(&[record_index as u8]),
                false,
                false,
            ));
        }
    }
    assert!(enqueue_pending(
        &mut pending,
        &mut pending_count,
        "connected-fifth".to_string(),
        frame(b"deliver"),
        false,
        false,
    ));
}

#[test]
fn removing_stream_state_rewinds_unacked_frame_and_prunes_request_id() {
    let config = TcpConfig {
        send_buffer: 8,
        ..TcpConfig::default()
    };
    let mut client = fips_tcp::Stack::new(config.clone(), 1);
    let mut server = fips_tcp::Stack::new(config, 2);
    assert!(server.listen(7369).is_ok());
    let Ok(id) = client.connect("server".to_owned(), 7369, 0) else {
        panic!("test connection should start");
    };
    for _ in 0..4 {
        for packet in client.drain_outbound() {
            assert!(server.input("client".to_owned(), &packet.bytes, 0).is_ok());
        }
        for packet in server.drain_outbound() {
            assert!(client.input("server".to_owned(), &packet.bytes, 0).is_ok());
        }
    }

    let peer = "server".to_owned();
    let mut pending = HashMap::new();
    let mut count = 0;
    assert!(enqueue_pending(
        &mut pending,
        &mut count,
        peer.clone(),
        frame(b"long-enough-to-be-partial"),
        false,
        false,
    ));
    let Some(record) = pending
        .get_mut(&peer)
        .and_then(|records| records.front_mut())
    else {
        panic!("queued record should exist");
    };
    let Ok((accepted, marker)) = client.write_with_marker(id, &record.bytes, 0) else {
        panic!("established stream should accept bytes");
    };
    record.offset += accepted;
    record.marker = Some(marker);
    assert_eq!(record.offset, 8);
    assert_eq!(client.marker_status(&marker), MarkerStatus::Pending);

    let mut connections = HashMap::from([(peer.clone(), id)]);
    let mut readers = HashMap::from([(id, RecordReader::new(1024))]);
    let mut requested = HashSet::from([id]);
    assert_eq!(
        remove_connection_state(
            &peer,
            &mut connections,
            &mut readers,
            &mut pending,
            &mut requested,
        ),
        Some(id)
    );
    assert!(connections.is_empty());
    assert!(readers.is_empty());
    assert!(requested.is_empty());
    assert!(pending
        .get(&peer)
        .and_then(|records| records.front())
        .is_some_and(|record| record.offset == 0 && record.marker.is_none()));
}

#[test]
fn disconnected_and_non_progressable_streams_require_retirement() {
    assert!(connection_requires_retirement(
        false,
        Some(State::Established)
    ));
    assert!(connection_requires_retirement(true, None));
    assert!(connection_requires_retirement(true, Some(State::TimeWait)));
    assert!(!connection_requires_retirement(true, Some(State::SynSent)));
    assert!(!connection_requires_retirement(
        true,
        Some(State::Established)
    ));
    assert!(!connection_requires_retirement(
        true,
        Some(State::CloseWait)
    ));
}

#[test]
fn sender_rejects_oversized_records_before_the_command_queue() {
    let (tx, rx) = flume::bounded(1);
    let sender = DeviceSyncTcpSender {
        tx,
        max_record_bytes: 4,
        dirty: Arc::new(Mutex::new(HashSet::new())),
        control_retry: Arc::new(Mutex::new(HashMap::new())),
    };
    let identity = fips_core::Identity::generate();
    let peer = PeerIdentity::from_pubkey_full(identity.pubkey_full());
    assert!(!sender.send_batch(peer, vec![vec![0; 5]]));
    assert!(rx.is_empty());
}

#[test]
fn full_command_queue_schedules_a_priority_resync_notice() {
    let (sender, _rx) = DeviceSyncTcpSender::test_channel(1, 1024);
    let identity = fips_core::Identity::generate();
    let peer = PeerIdentity::from_pubkey_full(identity.pubkey_full());
    assert!(sender.send_batch(peer, vec![b"accepted".to_vec()]));
    assert!(!sender.send_batch(peer, vec![b"recover me".to_vec()]));
    assert!(sender
        .dirty
        .lock()
        .is_ok_and(|peers| peers.contains(&peer.npub())));

    let mut pending = HashMap::new();
    let mut count = 0;
    enqueue_dirty_resyncs(&sender.dirty, &mut pending, &mut count, b"resync");
    assert_eq!(
        pending[&peer.npub()].front().unwrap().bytes,
        frame(b"resync")
    );
    assert!(sender.dirty.lock().is_ok_and(|peers| peers.is_empty()));
}

#[test]
fn snapshot_request_retries_outside_a_full_data_command_queue() {
    let (sender, _rx) = DeviceSyncTcpSender::test_channel(1, 1024);
    let identity = fips_core::Identity::generate();
    let peer = PeerIdentity::from_pubkey_full(identity.pubkey_full());
    assert!(sender.send_batch(peer, vec![b"fills data queue".to_vec()]));
    assert!(sender.send_control(
        peer,
        b"cursor request".to_vec(),
        Some((1, 20, "chat".to_string(), "id".to_string())),
    ));
    assert!(sender.send_control(peer, b"full request".to_vec(), None));
    assert!(sender.send_control(
        peer,
        b"later cursor".to_vec(),
        Some((1, 30, "chat".to_string(), "id".to_string())),
    ));
    assert!(sender.dirty.lock().is_ok_and(|peers| peers.is_empty()));

    let mut pending = HashMap::new();
    let mut count = 0;
    enqueue_control_retries(&sender.control_retry, &mut pending, &mut count);
    assert_eq!(
        pending[&peer.npub()].front().unwrap().bytes,
        frame(b"full request")
    );
    assert!(sender
        .control_retry
        .lock()
        .is_ok_and(|records| records.is_empty()));
}

async fn connected_loopback_tcp(
    service_port: u16,
) -> (
    Arc<FipsEndpoint>,
    FipsTcpEndpoint,
    PeerIdentity,
    ConnectionId,
    ConnectionId,
) {
    let endpoint = Arc::new(
        FipsEndpoint::builder()
            .without_system_tun()
            .bind()
            .await
            .expect("bind embedded endpoint"),
    );
    let local = PeerIdentity::from_npub(endpoint.npub()).expect("local identity");
    let mut tcp = FipsTcpEndpoint::bind(
        endpoint.clone(),
        service_port,
        TcpConfig::default(),
        0x1234_5678,
    )
    .await
    .expect("bind TCP service");
    let client = tcp.connect(local, 0).await.expect("connect loopback peer");
    for _ in 0..3 {
        tokio::time::timeout(Duration::from_secs(2), tcp.receive(0))
            .await
            .expect("handshake timed out")
            .expect("receive handshake");
    }
    let server = tcp.accept().expect("accept loopback connection");
    (endpoint, tcp, local, client, server)
}

#[tokio::test]
async fn record_queued_while_disconnected_is_delivered_after_connect() {
    let (endpoint, mut tcp, local, client, server) = connected_loopback_tcp(39_017).await;
    let peer = local.npub();
    let payload = b"queued before connect";
    let mut pending = HashMap::new();
    let mut pending_count = 0;
    assert!(enqueue_pending(
        &mut pending,
        &mut pending_count,
        peer.clone(),
        frame(payload),
        false,
        false,
    ));

    drain_writes(
        &peer,
        client,
        &mut pending,
        &mut pending_count,
        &mut tcp,
        10,
    )
    .await;
    tcp.receive(10).await.expect("receive queued record");
    tcp.receive(10).await.expect("receive acknowledgment");
    drain_writes(
        &peer,
        client,
        &mut pending,
        &mut pending_count,
        &mut tcp,
        10,
    )
    .await;

    let bytes = tcp.read(server, 1024, 10).await.expect("read stream");
    let records = RecordReader::new(1024)
        .push(&bytes)
        .expect("parse framed record");
    assert_eq!(records, vec![payload.to_vec()]);
    assert_eq!(pending_count, 0);
    assert!(pending.is_empty());
    endpoint.shutdown().await.expect("shutdown endpoint");
}

#[tokio::test]
async fn close_wait_is_retired_and_schedules_resync() {
    let (endpoint, mut tcp, local, client, server) = connected_loopback_tcp(39_018).await;
    tcp.close(client, 10)
        .await
        .expect("close client write side");
    tcp.receive(10).await.expect("receive client FIN");
    assert_eq!(tcp.state(server), Some(State::CloseWait));

    let peer = local.npub();
    let mut connections = HashMap::from([(peer.clone(), server)]);
    let mut readers = HashMap::from([(server, RecordReader::new(1024))]);
    let mut pending = HashMap::new();
    let mut pending_count = 0;
    let mut requested = HashSet::from([server]);
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    let (core_sender, _core_receiver) = flume::unbounded();
    progress_connections(
        &mut connections,
        &mut readers,
        &mut pending,
        &mut pending_count,
        &mut requested,
        &dirty,
        b"request",
        &core_sender,
        39_018,
        &mut tcp,
        10,
    )
    .await;

    assert!(connections.is_empty());
    assert!(readers.is_empty());
    assert!(requested.is_empty());
    assert_eq!(tcp.state(server), None);
    enqueue_dirty_resyncs(&dirty, &mut pending, &mut pending_count, b"resync");
    assert_eq!(pending[&peer].front().unwrap().bytes, frame(b"resync"));
    endpoint.shutdown().await.expect("shutdown endpoint");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_same_host_non_sibling_cannot_read_device_sync() {
    let rendezvous = reserve_local_rendezvous_addr();
    let first = local_rendezvous_endpoint(rendezvous).await;
    let second = local_rendezvous_endpoint(rendezvous).await;
    let first_identity = PeerIdentity::from_npub(first.npub()).expect("first identity");
    let second_identity = PeerIdentity::from_npub(second.npub()).expect("second identity");
    let (owner, attacker) = if comparison_key(&first_identity) >= comparison_key(&second_identity) {
        (first, second)
    } else {
        (second, first)
    };
    wait_for_authenticated_peer(&owner, attacker.npub()).await;
    wait_for_authenticated_peer(&attacker, owner.npub()).await;

    let (core_sender, _core_receiver) = flume::unbounded();
    let (_sender, owner_task) = start_device_sync_tcp(
        owner.clone(),
        HashSet::new(),
        39_019,
        1024,
        b"private device-sync request".to_vec(),
        b"private resync".to_vec(),
        core_sender,
    )
    .await
    .expect("start owner device sync");
    tokio::time::sleep(Duration::from_millis(500)).await;

    let owner_identity = PeerIdentity::from_npub(owner.npub()).expect("owner identity");
    let mut attacker_tcp =
        FipsTcpEndpoint::bind(attacker.clone(), 39_019, TcpConfig::default(), 0x8765_4321)
            .await
            .expect("bind attacker TCP endpoint");
    let stream = attacker_tcp
        .connect(owner_identity, 0)
        .await
        .expect("attempt unauthorized device-sync stream");
    let started = StdInstant::now();
    let deadline = started + Duration::from_secs(3);
    let mut received = Vec::new();
    while StdInstant::now() < deadline && attacker_tcp.state(stream).is_some() {
        let now = elapsed_millis(started);
        let _ = tokio::time::timeout(Duration::from_millis(100), attacker_tcp.receive(now)).await;
        if matches!(attacker_tcp.state(stream), Some(State::Established)) {
            received.extend(
                attacker_tcp
                    .read(stream, 1024, now)
                    .await
                    .unwrap_or_default(),
            );
        }
        let _ = attacker_tcp.poll(now).await;
    }

    assert!(
        received.is_empty(),
        "authenticated local non-sibling read private device-sync bytes: {received:?}"
    );
    assert_eq!(
        attacker_tcp.state(stream),
        None,
        "unauthorized device-sync stream must be reset"
    );
    owner_task.abort();
    attacker.shutdown().await.expect("shutdown attacker");
    owner.shutdown().await.expect("shutdown owner");
}
