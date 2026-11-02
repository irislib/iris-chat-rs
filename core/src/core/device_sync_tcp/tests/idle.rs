use super::*;

const PORT: u16 = 39_022;

struct RunningPeer {
    sender: DeviceSyncTcpSender,
    received: flume::Receiver<CoreMsg>,
    task: JoinHandle<()>,
}

impl Drop for RunningPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn endpoints() -> (Arc<FipsEndpoint>, Arc<FipsEndpoint>) {
    let address = reserve_local_rendezvous_addr();
    let a = local_rendezvous_endpoint(address).await;
    let b = local_rendezvous_endpoint(address).await;
    wait_for_authenticated_peer(&a, b.npub()).await;
    wait_for_authenticated_peer(&b, a.npub()).await;
    let a_id = PeerIdentity::from_npub(a.npub()).unwrap();
    let b_id = PeerIdentity::from_npub(b.npub()).unwrap();
    if comparison_key(&a_id) < comparison_key(&b_id) {
        (a, b)
    } else {
        (b, a)
    }
}

async fn start(endpoint: &Arc<FipsEndpoint>, peer: Option<&str>) -> RunningPeer {
    let (tx, received) = flume::unbounded();
    let (sender, task) = start_device_sync_tcp(
        endpoint.clone(),
        peer.map(str::to_owned).into_iter().collect(),
        PORT,
        1024,
        b"history".to_vec(),
        b"resync".to_vec(),
        tx,
    )
    .await
    .unwrap();
    RunningPeer {
        sender,
        received,
        task,
    }
}

async fn receive(peer: &RunningPeer, expected: &[u8]) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let CoreMsg::Internal(event) = peer.received.recv_async().await.unwrap() {
                if let InternalEvent::DeviceSyncPacket { data, .. } = *event {
                    assert_eq!(data, expected);
                    return;
                }
            }
        }
    })
    .await
    .expect("device-sync record must arrive");
}

fn iterations(peers: &[&RunningPeer]) -> Vec<usize> {
    peers
        .iter()
        .map(|peer| peer.sender.iterations.load(Ordering::Relaxed))
        .collect()
}

async fn assert_quiet(peers: &[&RunningPeer]) {
    // Await ACKs, then observe the real app loop rather than checking a timer
    // helper. The old 25 ms poll cannot produce this quiet interval.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let before = iterations(peers);
            tokio::time::sleep(Duration::from_millis(200)).await;
            if iterations(peers) == before {
                break;
            }
        }
    })
    .await
    .expect("settled device-sync loops must stop polling");
    let before = iterations(peers);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        iterations(peers),
        before,
        "idle loops must have no periodic work"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn established_streams_sleep_and_data_and_control_wake_them() {
    let (a, b) = endpoints().await;
    let first = start(&a, Some(b.npub())).await;
    let second = start(&b, Some(a.npub())).await;
    receive(&first, b"history").await;
    receive(&second, b"history").await;
    assert_quiet(&[&first, &second]).await;

    assert!(first.sender.send_batch(
        PeerIdentity::from_npub(b.npub()).unwrap(),
        vec![b"after idle".to_vec()]
    ));
    receive(&second, b"after idle").await;
    assert_quiet(&[&first, &second]).await;
    assert!(second.sender.send_control(
        PeerIdentity::from_npub(a.npub()).unwrap(),
        b"control after idle".to_vec(),
        None
    ));
    receive(&first, b"control after idle").await;
    assert_quiet(&[&first, &second]).await;

    first.sender.update_peers(HashSet::new());
    assert!(second.sender.send_batch(
        PeerIdentity::from_npub(a.npub()).unwrap(),
        vec![b"revoked".to_vec()]
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(350), first.received.recv_async())
            .await
            .is_err(),
        "removing a sibling must revoke an already idle connection"
    );
    drop((first, second));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_service_retries_handshake_at_transport_deadline() {
    let (a, b) = endpoints().await;
    // Consume and drop the actual first SYN. Only the lower-key endpoint
    // dials, so its transport deadline must retry without another app command.
    let blackhole = b.register_service_receiver(PORT).await.unwrap();
    let first = start(&a, Some(b.npub())).await;
    let mut dropped = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        blackhole.recv_batch_into(&mut dropped, 1),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        dropped[0].source_peer,
        PeerIdentity::from_npub(a.npub()).unwrap()
    );
    let syn = fips_tcp::wire::Segment::decode(dropped[0].data.as_slice()).unwrap();
    assert!(syn.flags.contains(fips_tcp::wire::Flags::SYN));
    assert!(first.received.is_empty());
    drop(blackhole);
    let second = start(&b, Some(a.npub())).await;
    receive(&first, b"history").await;
    receive(&second, b"history").await;
    assert_quiet(&[&first, &second]).await;
    drop((first, second));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dormant_service_wakes_for_peer_changes_and_closes_with_its_sender() {
    let (a, b) = endpoints().await;
    let first = start(&a, None).await;
    let second = start(&b, None).await;
    assert_quiet(&[&first, &second]).await;
    second
        .sender
        .update_peers(HashSet::from([a.npub().to_owned()]));
    first
        .sender
        .update_peers(HashSet::from([b.npub().to_owned()]));
    receive(&first, b"history").await;
    receive(&second, b"history").await;
    drop((first, second));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();

    let endpoint = local_rendezvous_endpoint(reserve_local_rendezvous_addr()).await;
    let (tx, _rx) = flume::unbounded();
    let (sender, mut task) = start_device_sync_tcp(
        endpoint.clone(),
        HashSet::new(),
        PORT,
        1024,
        b"history".to_vec(),
        b"resync".to_vec(),
        tx,
    )
    .await
    .unwrap();
    drop(sender);
    let result = tokio::time::timeout(Duration::from_secs(1), &mut task).await;
    task.abort();
    endpoint.shutdown().await.unwrap();
    result
        .expect("closed command channel must stop the loop")
        .unwrap();
}
