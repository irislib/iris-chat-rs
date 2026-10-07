use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::Duration,
};

const TOKEN: &str = "0102030405060708010203040506070801020304050607080102030405060708";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_file_listener_sleeps_but_commands_wake_it_immediately() {
    use std::future::{poll_fn, Future};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    let endpoint = Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    );
    let tcp = FipsTcpEndpoint::bind(endpoint.clone(), PORT, Config::default(), 1)
        .await
        .unwrap();
    let local = PeerIdentity::from_npub(endpoint.npub()).unwrap();
    let (commands, rx) = flume::bounded(32);
    let (tx, events) = flume::unbounded();
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    let task = tokio::spawn(async move {
        let future = run(tcp, local, rx, tx, Arc::new(RwLock::new(HashMap::new())));
        tokio::pin!(future);
        poll_fn(|cx| {
            observed.fetch_add(1, Ordering::Relaxed);
            future.as_mut().poll(cx)
        })
        .await;
    });
    tokio::time::sleep(Duration::from_millis(350)).await;
    let idle_polls = polls.load(Ordering::Relaxed);
    assert!(
        idle_polls <= 8,
        "idle file listener was polled {idle_polls} times"
    );
    commands
        .send(Command::Cancel("wake-idle-listener".into()))
        .unwrap();
    let event = tokio::time::timeout(Duration::from_millis(100), events.recv_async())
        .await
        .expect("an idle listener must wake for commands")
        .unwrap();
    assert!(
        matches!(event, DirectFileEvent::Cancelled { transfer_id, .. }
        if transfer_id == "wake-idle-listener")
    );
    drop(commands);
    task.await.unwrap();
    endpoint.shutdown().await.unwrap();
}

struct Peer {
    endpoint: Arc<FipsEndpoint>,
    identity: PeerIdentity,
    sender: DirectFileSender,
    events: flume::Receiver<DirectFileEvent>,
    task: JoinHandle<()>,
}
impl Peer {
    async fn new(address: SocketAddrV4) -> Self {
        let mut config = fips_core::Config::new();
        config.node.control.enabled = false;
        config.node.discovery.local.rendezvous_addr = address;
        config.node.discovery.local.retry_interval_ms = 20;
        config.node.discovery.lan.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.routing.mode = fips_core::config::RoutingMode::ReplyLearned;
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .config(config)
                .local_rendezvous()
                .without_system_tun()
                .bind()
                .await
                .unwrap(),
        );
        let identity = PeerIdentity::from_npub(endpoint.npub()).unwrap();
        let (tx, events) = flume::unbounded();
        let (sender, task) = start_direct_file_tcp(endpoint.clone(), tx).await.unwrap();
        Self {
            endpoint,
            identity,
            sender,
            events,
            task,
        }
    }
    async fn wait_for(&self, other: &Peer) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .endpoint
                    .peers()
                    .await
                    .unwrap()
                    .iter()
                    .any(|p| p.connected && p.npub == other.endpoint.npub())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("FIPS authentication timed out");
    }
    async fn event(&self, predicate: impl Fn(&DirectFileEvent) -> bool) -> DirectFileEvent {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let event = self.events.recv_async().await.unwrap();
                if predicate(&event) {
                    return event;
                }
            }
        })
        .await
        .expect("file transfer event timed out")
    }
    async fn stop(self) {
        self.task.abort();
        self.endpoint.shutdown().await.unwrap();
    }
}
fn address() -> SocketAddrV4 {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    match socket.local_addr().unwrap() {
        SocketAddr::V4(a) => a,
        _ => unreachable!(),
    }
}
fn source(directory: &std::path::Path, name: &str, bytes: &[u8]) -> TransferFile {
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    TransferFile {
        filename: name.into(),
        size_bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        path,
    }
}
async fn pair() -> (Peer, Peer) {
    let address = address();
    let a = Peer::new(address).await;
    let b = Peer::new(address).await;
    a.wait_for(&b).await;
    b.wait_for(&a).await;
    (a, b)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fips_multiple_files_are_sent_only_after_recipient_accepts() {
    let (a, b) = pair().await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let content = vec![0xab; 180_123];
    let files = vec![
        source(source_dir.path(), "empty.txt", b""),
        source(source_dir.path(), "photo.bin", &content),
        source(source_dir.path(), "notes.txt", b"hello"),
    ];
    a.sender
        .register_offer(
            "multi".into(),
            TOKEN.into(),
            vec![b.identity],
            files.clone(),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(a.events.is_empty());
    assert!(b.events.is_empty());
    assert_eq!(
        fs::read_dir(destination.path()).unwrap().count(),
        0,
        "no bytes are received before acceptance"
    );
    b.sender
        .receive(
            "multi".into(),
            TOKEN.into(),
            a.identity,
            files,
            destination.path().into(),
        )
        .unwrap();
    let accepted = a
        .event(|e| matches!(e, DirectFileEvent::Accepted { .. }))
        .await;
    assert!(
        matches!(accepted, DirectFileEvent::Accepted { peer, .. } if peer == peer_hex(b.identity))
    );
    let completed = b
        .event(|e| {
            matches!(
                e,
                DirectFileEvent::Completed { .. } | DirectFileEvent::Failed { .. }
            )
        })
        .await;
    let DirectFileEvent::Completed { local_paths, .. } = completed else {
        panic!("{completed:?}")
    };
    assert_eq!(local_paths.len(), 3);
    assert_eq!(fs::read(&local_paths[0]).unwrap(), b"");
    assert_eq!(fs::read(&local_paths[1]).unwrap(), content);
    assert_eq!(fs::read(&local_paths[2]).unwrap(), b"hello");
    assert!(matches!(
        a.event(|e| matches!(
            e,
            DirectFileEvent::Completed { .. } | DirectFileEvent::Failed { .. }
        ))
        .await,
        DirectFileEvent::Completed { .. }
    ));
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_unauthorized_device_and_bad_token_cannot_claim_offer() {
    let rendezvous = address();
    let a = Peer::new(rendezvous).await;
    let b = Peer::new(rendezvous).await;
    let outsider = Peer::new(rendezvous).await;
    a.wait_for(&b).await;
    b.wait_for(&a).await;
    a.wait_for(&outsider).await;
    outsider.wait_for(&a).await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let files = vec![source(source_dir.path(), "secret.txt", b"recipient only")];
    a.sender
        .register_offer(
            "private".into(),
            TOKEN.into(),
            vec![b.identity],
            files.clone(),
        )
        .unwrap();
    outsider
        .sender
        .receive(
            "private".into(),
            TOKEN.into(),
            a.identity,
            files.clone(),
            destination.path().into(),
        )
        .unwrap();
    outsider
        .event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    assert!(
        a.events.is_empty(),
        "unauthorized request did not claim offer"
    );
    assert!(!destination.path().join("Iris files private").exists());
    b.sender
        .receive(
            "private".into(),
            "ff".repeat(32),
            a.identity,
            files.clone(),
            destination.path().into(),
        )
        .unwrap();
    b.event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    assert!(a.events.is_empty(), "bad token did not claim offer");
    b.sender
        .receive(
            "private".into(),
            TOKEN.into(),
            a.identity,
            files,
            destination.path().into(),
        )
        .unwrap();
    assert!(matches!(
        b.event(|e| matches!(
            e,
            DirectFileEvent::Completed { .. } | DirectFileEvent::Failed { .. }
        ))
        .await,
        DirectFileEvent::Completed { .. }
    ));
    a.stop().await;
    b.stop().await;
    outsider.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_offer_cannot_be_replayed_by_another_authorized_device() {
    let rendezvous = address();
    let a = Peer::new(rendezvous).await;
    let b = Peer::new(rendezvous).await;
    let c = Peer::new(rendezvous).await;
    a.wait_for(&b).await;
    b.wait_for(&a).await;
    a.wait_for(&c).await;
    c.wait_for(&a).await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let other_destination = tempfile::tempdir().unwrap();
    let files = vec![source(
        source_dir.path(),
        "note.txt",
        b"one accepting device",
    )];
    a.sender
        .register_offer(
            "once".into(),
            TOKEN.into(),
            vec![a.identity, b.identity, c.identity],
            files.clone(),
        )
        .unwrap();
    b.sender
        .receive(
            "once".into(),
            TOKEN.into(),
            a.identity,
            files.clone(),
            destination.path().into(),
        )
        .unwrap();
    b.event(|e| matches!(e, DirectFileEvent::Completed { .. }))
        .await;
    c.sender
        .receive(
            "once".into(),
            TOKEN.into(),
            a.identity,
            files,
            other_destination.path().into(),
        )
        .unwrap();
    c.event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    assert_eq!(fs::read_dir(other_destination.path()).unwrap().count(), 0);
    a.stop().await;
    b.stop().await;
    c.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_removes_partial_files_and_notifies_both_devices() {
    let (a, b) = pair().await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let files = vec![source(
        source_dir.path(),
        "large.bin",
        &vec![7; 16 * 1024 * 1024],
    )];
    a.sender
        .register_offer(
            "cancel".into(),
            TOKEN.into(),
            vec![b.identity],
            files.clone(),
        )
        .unwrap();
    b.sender
        .receive(
            "cancel".into(),
            TOKEN.into(),
            a.identity,
            files,
            destination.path().into(),
        )
        .unwrap();
    a.event(|e| matches!(e, DirectFileEvent::Accepted { .. }))
        .await;
    b.sender.cancel("cancel").unwrap();
    b.event(|e| matches!(e, DirectFileEvent::Cancelled { .. }))
        .await;
    a.event(|e| matches!(e, DirectFileEvent::Cancelled { .. }))
        .await;
    assert!(!destination.path().join("Iris files cancel").exists());
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_accepts_deliver_to_exactly_one_authorized_device() {
    let rendezvous = address();
    let a = Peer::new(rendezvous).await;
    let b = Peer::new(rendezvous).await;
    let c = Peer::new(rendezvous).await;
    a.wait_for(&b).await;
    b.wait_for(&a).await;
    a.wait_for(&c).await;
    c.wait_for(&a).await;
    let source_dir = tempfile::tempdir().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let files = vec![source(source_dir.path(), "one.bin", &vec![9; 120_000])];
    a.sender
        .register_offer(
            "race".into(),
            TOKEN.into(),
            vec![b.identity, c.identity],
            files.clone(),
        )
        .unwrap();
    b.sender
        .receive(
            "race".into(),
            TOKEN.into(),
            a.identity,
            files.clone(),
            first.path().into(),
        )
        .unwrap();
    c.sender
        .receive(
            "race".into(),
            TOKEN.into(),
            a.identity,
            files,
            second.path().into(),
        )
        .unwrap();
    let terminal = |e: &DirectFileEvent| {
        matches!(
            e,
            DirectFileEvent::Completed { .. } | DirectFileEvent::Failed { .. }
        )
    };
    let (b_result, c_result) = tokio::join!(b.event(terminal), c.event(terminal));
    let b_won = matches!(b_result, DirectFileEvent::Completed { .. });
    let c_won = matches!(c_result, DirectFileEvent::Completed { .. });
    assert_ne!(b_won, c_won, "exactly one accepting device receives bytes");
    assert_eq!(first.path().join("Iris files race").exists(), b_won);
    assert_eq!(second.path().join("Iris files race").exists(), c_won);
    a.stop().await;
    b.stop().await;
    c.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn roster_revocation_aborts_active_transfer_and_removes_partial_files() {
    let (a, b) = pair().await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let files = vec![source(
        source_dir.path(),
        "private.bin",
        &vec![3; 2 * 1024 * 1024],
    )];
    a.sender
        .register_offer(
            "revoke".into(),
            TOKEN.into(),
            vec![b.identity],
            files.clone(),
        )
        .unwrap();
    b.sender
        .receive(
            "revoke".into(),
            TOKEN.into(),
            a.identity,
            files,
            destination.path().into(),
        )
        .unwrap();
    a.event(|e| matches!(e, DirectFileEvent::Accepted { .. }))
        .await;
    a.sender.restrict_offer("revoke", Vec::new()).unwrap();
    a.event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    b.event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    assert!(!destination.path().join("Iris files revoke").exists());
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hash_mismatch_fails_and_removes_received_bytes() {
    let (a, b) = pair().await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let mut files = vec![source(source_dir.path(), "broken.txt", b"actual file")];
    files[0].sha256 = "00".repeat(32);
    a.sender
        .register_offer("hash".into(), TOKEN.into(), vec![b.identity], files.clone())
        .unwrap();
    b.sender
        .receive(
            "hash".into(),
            TOKEN.into(),
            a.identity,
            files,
            destination.path().into(),
        )
        .unwrap();
    let failed = b
        .event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    assert!(
        matches!(failed, DirectFileEvent::Failed { error, .. } if error.contains("did not match"))
    );
    assert!(!destination.path().join("Iris files hash").exists());
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decline_never_reads_source_files_and_revokes_the_offer() {
    let (a, b) = pair().await;
    let source_dir = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let files = vec![source(source_dir.path(), "declined.txt", b"not sent")];
    a.sender
        .register_offer(
            "decline".into(),
            TOKEN.into(),
            vec![b.identity],
            files.clone(),
        )
        .unwrap();
    fs::remove_file(&files[0].path).unwrap();
    b.sender
        .decline("decline".into(), TOKEN.into(), a.identity)
        .unwrap();
    a.event(|e| matches!(e, DirectFileEvent::Declined { .. }))
        .await;
    b.event(|e| matches!(e, DirectFileEvent::Declined { .. }))
        .await;
    b.sender
        .receive(
            "decline".into(),
            TOKEN.into(),
            a.identity,
            files,
            destination.path().into(),
        )
        .unwrap();
    b.event(|e| matches!(e, DirectFileEvent::Failed { .. }))
        .await;
    assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
    a.stop().await;
    b.stop().await;
}

#[test]
fn bounded_frames_and_safe_filenames() {
    let mut reader = Reader::default();
    let framed = wire::data(vec![42; 19]);
    assert!(reader.push(&framed[..2]).unwrap().is_empty());
    assert_eq!(
        reader.push(&framed[2..]).unwrap()[0],
        [vec![1], vec![42; 19]].concat()
    );
    assert!(reader.push(&u32::MAX.to_be_bytes()).is_err());
    let mut file = TransferFile {
        filename: "../escape".into(),
        size_bytes: 0,
        sha256: "00".repeat(32),
        path: PathBuf::new(),
    };
    assert!(files::validate(&[file.clone()]).is_err());
    file.filename = "ok.txt".into();
    assert!(files::validate(&[file]).is_ok());
    assert!(wire::validate_claim("../escape", TOKEN).is_err());
}
