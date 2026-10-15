use super::*;

struct Peers {
    native: Arc<FipsEndpoint>,
    browser: Arc<FipsEndpoint>,
    local: PeerIdentity,
    peer: PeerIdentity,
    native_tcp: FipsTcpEndpoint,
    browser_tcp: FipsTcpEndpoint,
    now: u64,
}

impl Peers {
    async fn new() -> Self {
        let address = reserve_local_rendezvous_addr();
        let a = local_rendezvous_endpoint(address).await;
        let b = local_rendezvous_endpoint(address).await;
        let a_id = PeerIdentity::from_npub(a.npub()).unwrap();
        let b_id = PeerIdentity::from_npub(b.npub()).unwrap();
        let (native, browser) = if comparison_key(&a_id) < comparison_key(&b_id) {
            (a, b)
        } else {
            (b, a)
        };
        wait_for_authenticated_peer(&native, browser.npub()).await;
        wait_for_authenticated_peer(&browser, native.npub()).await;
        let native_tcp = FipsTcpEndpoint::bind(
            native.clone(),
            39_021,
            TcpConfig {
                send_buffer: 8,
                ..TcpConfig::default()
            },
            1,
        )
        .await
        .unwrap();
        let browser_tcp = FipsTcpEndpoint::bind(browser.clone(), 39_021, TcpConfig::default(), 2)
            .await
            .unwrap();
        Self {
            local: PeerIdentity::from_npub(native.npub()).unwrap(),
            peer: PeerIdentity::from_npub(browser.npub()).unwrap(),
            native,
            browser,
            native_tcp,
            browser_tcp,
            now: 0,
        }
    }

    async fn pump(&mut self) {
        self.now += 25;
        self.native_tcp.poll(self.now).await.unwrap();
        self.browser_tcp.poll(self.now).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_millis(20), self.native_tcp.receive(self.now))
            .await;
        let _ = tokio::time::timeout(
            Duration::from_millis(20),
            self.browser_tcp.receive(self.now),
        )
        .await;
    }

    async fn original_stream(&mut self) -> (ConnectionId, ConnectionId) {
        let native = self.native_tcp.connect(self.peer, self.now).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.native_tcp.state(native) != Some(State::Established) {
                self.pump().await;
            }
        })
        .await
        .expect("original native stream handshake");
        (
            native,
            self.browser_tcp.accept().expect("original browser stream"),
        )
    }

    async fn incoming_stream(&mut self) -> ConnectionId {
        let browser = self
            .browser_tcp
            .connect(self.local, self.now)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.browser_tcp.state(browser) != Some(State::Established) {
                self.pump().await;
            }
            self.pump().await; // Deliver the final ACK before app-level admission.
        })
        .await
        .expect("fresh authenticated sibling handshake");
        browser
    }

    async fn shutdown(self) {
        self.native.shutdown().await.unwrap();
        self.browser.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_recovery_probe_redials_canonical_stream_and_delivers_pending_records() {
    let mut pair = Peers::new().await;
    let (old, old_browser) = pair.original_stream().await;
    let peer = pair.peer.npub();
    let mut connections = HashMap::from([(peer.clone(), old)]);
    let mut readers = HashMap::from([(old, RecordReader::new(1024))]);
    let mut pending = HashMap::new();
    let mut pending_count = 0;
    let mut requested = HashSet::from([old]);
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    let (core_sender, core_receiver) = flume::unbounded();

    // Leave a partial outbound frame and an incomplete inbound header behind.
    // A browser reload does not necessarily send FIN through an intermediate seed.
    assert!(enqueue_pending(
        &mut pending,
        &mut pending_count,
        peer.clone(),
        frame(b"retained after reload"),
        false,
        false
    ));
    drain_writes(
        &peer,
        old,
        &mut pending,
        &mut pending_count,
        &mut pair.native_tcp,
        pair.now,
    )
    .await;
    assert_eq!(pending[&peer][0].offset, 8);
    pair.browser_tcp
        .write(old_browser, &frame(b"old frame")[..3], pair.now)
        .await
        .unwrap();
    pair.pump().await;
    drain_reads(
        &peer,
        old,
        &mut readers,
        &core_sender,
        39_021,
        &mut pair.native_tcp,
        pair.now,
    )
    .await;
    assert!(core_receiver.is_empty());

    pair.incoming_stream().await;
    assert_eq!(
        pair.native_tcp.state(old),
        Some(State::Established),
        "fixture retains the obsolete Established stream until explicit replacement"
    );
    // Complete the old buffered frame, then leave another partial header.
    // Production drains readable canonical bytes before admitting a probe, so
    // TCP-ACKed application records must survive while the partial header resets.
    let mut old_tail = frame(b"old frame")[3..].to_vec();
    old_tail.extend_from_slice(&frame(b"unfinished old frame")[..3]);
    pair.browser_tcp
        .write(old_browser, &old_tail, pair.now)
        .await
        .unwrap();
    pair.pump().await;
    progress_connections(
        &mut connections,
        &mut readers,
        &mut pending,
        &mut pending_count,
        &mut requested,
        &dirty,
        b"inventory",
        &core_sender,
        39_021,
        &mut pair.native_tcp,
        pair.now,
    )
    .await;
    let CoreMsg::Internal(event) = core_receiver
        .try_recv()
        .expect("old complete frame is drained")
    else {
        panic!("internal packet")
    };
    let InternalEvent::DeviceSyncPacket { data, .. } = *event else {
        panic!("device sync packet")
    };
    assert_eq!(data, b"old frame");
    assert!(core_receiver.is_empty());
    accept_connections(
        &pair.local,
        &HashSet::from([peer.clone()]),
        &mut connections,
        &mut readers,
        &mut pending,
        &mut requested,
        &dirty,
        &mut pair.native_tcp,
        1024,
    )
    .await;
    assert!(
        connections.is_empty(),
        "a recovery probe only wakes the canonical dialer"
    );
    assert!(readers.is_empty());
    assert!(!readers.contains_key(&old) && !requested.contains(&old));
    assert_eq!(pair.native_tcp.state(old), None);
    assert_eq!(pending[&peer][0].offset, 0);
    assert!(pending[&peer][0].marker.is_none());
    assert!(dirty.lock().unwrap().contains(&peer));

    pair.now += CONNECTION_RETRY_MILLIS;
    reconcile_connections(
        &pair.local,
        &HashSet::from([peer.clone()]),
        &mut connections,
        &mut readers,
        &mut pending,
        &mut requested,
        &dirty,
        &mut pair.native_tcp,
        1024,
        pair.now,
    )
    .await;
    let fresh = connections[&peer];
    assert_ne!(fresh, old);
    tokio::time::timeout(Duration::from_secs(5), async {
        while pair.native_tcp.state(fresh) != Some(State::Established) {
            pair.pump().await;
        }
    })
    .await
    .expect("recovery probe causes a new canonical handshake");
    let fresh_browser = pair.browser_tcp.accept().expect("new canonical stream");
    assert_eq!(connections.len(), 1);
    assert_eq!(readers.len(), 1);
    enqueue_dirty_resyncs(&dirty, &mut pending, &mut pending_count, b"resync");
    pair.browser_tcp
        .write(fresh_browser, &frame(b"updated preference"), pair.now)
        .await
        .unwrap();
    let mut browser_reader = RecordReader::new(1024);
    let mut browser_records = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            pair.pump().await;
            progress_connections(
                &mut connections,
                &mut readers,
                &mut pending,
                &mut pending_count,
                &mut requested,
                &dirty,
                b"fresh inventory request",
                &core_sender,
                39_021,
                &mut pair.native_tcp,
                pair.now,
            )
            .await;
            let bytes = pair
                .browser_tcp
                .read(fresh_browser, 1024, pair.now)
                .await
                .unwrap();
            browser_records.extend(browser_reader.push(&bytes).unwrap());
            if pending_count == 0 && !core_receiver.is_empty() {
                break;
            }
        }
    })
    .await
    .expect("replacement stream transfers both directions without expiry or another reconnect");
    assert_eq!(
        browser_records
            .iter()
            .filter(|record| record.as_slice() == b"fresh inventory request")
            .count(),
        1
    );
    assert!(browser_records
        .iter()
        .any(|record| record == b"retained after reload"));
    assert!(browser_records.iter().any(|record| record == b"resync"));
    let CoreMsg::Internal(event) = core_receiver.try_recv().unwrap() else {
        panic!("internal packet")
    };
    let InternalEvent::DeviceSyncPacket {
        source_pubkey_hex,
        data,
        ..
    } = *event
    else {
        panic!("device sync packet")
    };
    assert_eq!(source_pubkey_hex, pair.peer.pubkey().to_string());
    assert_eq!(data, b"updated preference");
    assert!(core_receiver.is_empty());
    assert_eq!(requested, HashSet::from([fresh]));
    pair.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_stream_preserves_canonical_syn_sent_tie_break() {
    let mut pair = Peers::new().await;
    pair.incoming_stream().await;
    // Start the canonical outbound connection only after the competing inbound
    // handshake completes; do not deliver this SYN before app admission.
    let outbound = pair.native_tcp.connect(pair.peer, pair.now).await.unwrap();
    assert_eq!(pair.native_tcp.state(outbound), Some(State::SynSent));
    let peer = pair.peer.npub();
    let mut connections = HashMap::from([(peer.clone(), outbound)]);
    let mut readers = HashMap::from([(outbound, RecordReader::new(1024))]);
    let mut pending = HashMap::new();
    let mut requested = HashSet::new();
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    accept_connections(
        &pair.local,
        &HashSet::from([peer.clone()]),
        &mut connections,
        &mut readers,
        &mut pending,
        &mut requested,
        &dirty,
        &mut pair.native_tcp,
        1024,
    )
    .await;
    assert_eq!(connections, HashMap::from([(peer, outbound)]));
    assert_eq!(pair.native_tcp.state(outbound), Some(State::SynSent));
    assert!(dirty.lock().unwrap().is_empty());
    pair.shutdown().await;
}

struct RunningApp {
    connections: HashMap<String, ConnectionId>,
    readers: HashMap<ConnectionId, RecordReader>,
    pending: HashMap<String, VecDeque<PendingRecord>>,
    pending_count: usize,
    requested: HashSet<ConnectionId>,
    dirty: Arc<Mutex<HashSet<String>>>,
    sender: Sender<CoreMsg>,
    receiver: flume::Receiver<CoreMsg>,
    records: Vec<Vec<u8>>,
    last_retry: u64,
}

impl RunningApp {
    fn new(peer: &PeerIdentity, outbound: Option<ConnectionId>, record: &[u8]) -> Self {
        let (sender, receiver) = flume::unbounded();
        let mut app = Self {
            connections: outbound.map(|id| (peer.npub(), id)).into_iter().collect(),
            readers: outbound
                .map(|id| (id, RecordReader::new(1024)))
                .into_iter()
                .collect(),
            pending: HashMap::new(),
            pending_count: 0,
            requested: HashSet::new(),
            dirty: Arc::new(Mutex::new(HashSet::new())),
            sender,
            receiver,
            records: Vec::new(),
            last_retry: 0,
        };
        assert!(enqueue_pending(
            &mut app.pending,
            &mut app.pending_count,
            peer.npub(),
            frame(record),
            false,
            false
        ));
        app
    }

    async fn step(
        &mut self,
        local: &PeerIdentity,
        peer: &PeerIdentity,
        tcp: &mut FipsTcpEndpoint,
        now: u64,
    ) {
        let allowed = HashSet::from([peer.npub()]);
        enqueue_dirty_resyncs(
            &self.dirty,
            &mut self.pending,
            &mut self.pending_count,
            b"resync",
        );
        if now.saturating_sub(self.last_retry) >= CONNECTION_RETRY_MILLIS {
            self.last_retry = now;
            reconcile_connections(
                local,
                &allowed,
                &mut self.connections,
                &mut self.readers,
                &mut self.pending,
                &mut self.requested,
                &self.dirty,
                tcp,
                1024,
                now,
            )
            .await;
        }
        // Preserve run_device_sync_tcp's production order: write/read progress
        // precedes admission, so requested is already set on an observed stream.
        tcp.poll(now).await.unwrap();
        progress_connections(
            &mut self.connections,
            &mut self.readers,
            &mut self.pending,
            &mut self.pending_count,
            &mut self.requested,
            &self.dirty,
            b"inventory",
            &self.sender,
            39_021,
            tcp,
            now,
        )
        .await;
        accept_connections(
            local,
            &allowed,
            &mut self.connections,
            &mut self.readers,
            &mut self.pending,
            &mut self.requested,
            &self.dirty,
            tcp,
            1024,
        )
        .await;
        for message in self.receiver.try_iter() {
            let CoreMsg::Internal(event) = message else {
                panic!("internal packet")
            };
            let InternalEvent::DeviceSyncPacket { data, .. } = *event else {
                panic!("device sync packet")
            };
            self.records.push(data);
        }
    }

    fn received(&self, record: &[u8]) -> usize {
        self.records
            .iter()
            .filter(|value| value.as_slice() == record)
            .count()
    }
}

async fn crossing_converges(observe_outbound_before_probe: bool, browser_steps_first: bool) {
    let mut pair = Peers::new().await;
    let native_outbound = pair.native_tcp.connect(pair.peer, pair.now).await.unwrap();
    let mut native_app = RunningApp::new(&pair.peer, Some(native_outbound), b"native preference");
    if observe_outbound_before_probe {
        tokio::time::timeout(Duration::from_secs(5), async {
            while pair.native_tcp.state(native_outbound) != Some(State::Established) {
                pair.pump().await;
            }
        })
        .await
        .expect("canonical outbound handshake before the delayed probe");
        native_app
            .step(&pair.local, &pair.peer, &mut pair.native_tcp, pair.now)
            .await;
        assert!(native_app.requested.contains(&native_outbound));
    }
    let browser_outbound = pair
        .browser_tcp
        .connect(pair.local, pair.now)
        .await
        .unwrap();
    // A recovery probe carries no application records. The passive sibling
    // keeps its queue until a canonical incoming stream is admitted.
    let mut browser_app = RunningApp::new(&pair.local, None, b"browser preference");
    tokio::time::timeout(Duration::from_secs(5), async {
        while pair.native_tcp.state(native_outbound) != Some(State::Established)
            || pair.browser_tcp.state(browser_outbound) != Some(State::Established)
        {
            pair.pump().await;
        }
        pair.pump().await;
    })
    .await
    .expect("both crossing handshakes complete before probe admission");
    if browser_steps_first {
        browser_app
            .step(&pair.peer, &pair.local, &mut pair.browser_tcp, pair.now)
            .await;
    }
    native_app
        .step(&pair.local, &pair.peer, &mut pair.native_tcp, pair.now)
        .await;
    assert!(
        native_app.connections.is_empty(),
        "the higher-key probe wakes the canonical dialer without becoming an app stream"
    );
    if !browser_steps_first {
        browser_app
            .step(&pair.peer, &pair.local, &mut pair.browser_tcp, pair.now)
            .await;
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            pair.pump().await;
            if browser_steps_first {
                browser_app
                    .step(&pair.peer, &pair.local, &mut pair.browser_tcp, pair.now)
                    .await;
                native_app
                    .step(&pair.local, &pair.peer, &mut pair.native_tcp, pair.now)
                    .await;
            } else {
                native_app
                    .step(&pair.local, &pair.peer, &mut pair.native_tcp, pair.now)
                    .await;
                browser_app
                    .step(&pair.peer, &pair.local, &mut pair.browser_tcp, pair.now)
                    .await;
            }
            if native_app.pending_count == 0
                && browser_app.pending_count == 0
                && native_app.received(b"browser preference") > 0
                && browser_app.received(b"native preference") > 0
            {
                break;
            }
        }
    })
    .await
    .expect("normal retry converges both queues after crossing handshakes");
    let settled_native = native_app.connections.clone();
    let settled_browser = browser_app.connections.clone();
    // Cross another normal retry interval after every frame is acknowledged:
    // late handshake/reset traffic must not churn the settled canonical stream.
    for _ in 0..12 {
        pair.pump().await;
        native_app
            .step(&pair.local, &pair.peer, &mut pair.native_tcp, pair.now)
            .await;
        browser_app
            .step(&pair.peer, &pair.local, &mut pair.browser_tcp, pair.now)
            .await;
    }
    assert_eq!(native_app.connections, settled_native);
    assert_eq!(browser_app.connections, settled_browser);
    assert_eq!(native_app.pending_count, 0);
    assert_eq!(browser_app.pending_count, 0);
    assert_eq!(native_app.received(b"browser preference"), 1);
    assert_eq!(browser_app.received(b"native preference"), 1);
    assert_eq!(native_app.connections.len(), 1);
    assert_eq!(browser_app.connections.len(), 1);
    assert_eq!(native_app.readers.len(), 1);
    assert_eq!(browser_app.readers.len(), 1);
    let native = native_app.connections[&pair.peer.npub()];
    let browser = browser_app.connections[&pair.local.npub()];
    let (native_port, native_remote_port) = pair.native_tcp.ports(native).unwrap();
    assert_eq!(native_remote_port, 39_021);
    assert_eq!(pair.browser_tcp.ports(browser), Some((39_021, native_port)));
    pair.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crossing_established_streams_converge_in_both_app_poll_orders() {
    crossing_converges(false, false).await;
    crossing_converges(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_probe_after_outbound_request_converges_in_both_app_poll_orders() {
    crossing_converges(true, false).await;
    crossing_converges(true, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unapproved_fresh_stream_cannot_replace_retained_connection() {
    let mut pair = Peers::new().await;
    let (old, _) = pair.original_stream().await;
    pair.incoming_stream().await;
    let peer = pair.peer.npub();
    let mut connections = HashMap::from([(peer.clone(), old)]);
    let mut readers = HashMap::from([(old, RecordReader::new(1024))]);
    let mut pending = HashMap::new();
    let mut requested = HashSet::from([old]);
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    accept_connections(
        &pair.local,
        &HashSet::new(),
        &mut connections,
        &mut readers,
        &mut pending,
        &mut requested,
        &dirty,
        &mut pair.native_tcp,
        1024,
    )
    .await;
    assert_eq!(connections, HashMap::from([(peer, old)]));
    assert_eq!(pair.native_tcp.state(old), Some(State::Established));
    assert_eq!(requested, HashSet::from([old]));
    assert!(dirty.lock().unwrap().is_empty());
    pair.shutdown().await;
}
