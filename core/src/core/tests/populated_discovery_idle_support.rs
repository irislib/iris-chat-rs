struct MeasuredProtocolStorage {
    inner: Arc<dyn StorageAdapter>,
    writes: Arc<AtomicUsize>,
}

impl StorageAdapter for MeasuredProtocolStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }
    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.put(key, value)
    }
    fn put_checkpoint(
        &self,
        key: &str,
        value: &iris_chat_protocol::StorageCheckpoint,
    ) -> StorageResult<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.put_checkpoint(key, value)
    }
    fn del(&self, key: &str) -> StorageResult<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.del(key)
    }
}

fn measure_core_writes(core: &mut AppCore) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let app_writes = Arc::new(AtomicUsize::new(0));
    let protocol_writes = Arc::new(AtomicUsize::new(0));
    let app_counter = app_writes.clone();
    core.app_store.shared().lock().unwrap().update_hook(Some(
        move |_, _: &str, table: &str, _: i64| {
            if table != "ndr_kv" {
                app_counter.fetch_add(1, Ordering::SeqCst);
            }
        },
    ));
    let logged = core.logged_in.as_ref().unwrap();
    let owner = logged.owner_keys.clone().unwrap();
    let device = logged.device_keys.clone();
    let storage = Arc::new(MeasuredProtocolStorage {
        inner: Arc::new(super::super::storage::SqliteStorageAdapter::new(
            core.app_store.shared(),
            owner.public_key().to_hex(),
            device.public_key().to_hex(),
        )),
        writes: protocol_writes.clone(),
    });
    install_test_protocol_engine(core, &owner, &device, storage, None, None);
    (app_writes, protocol_writes)
}

// Use production routing/batch limits, and allow the synchronous core to
// enter its runtime exactly as it does on its dedicated production thread.
fn dispatch_pending(core: &mut AppCore, incoming: &flume::Receiver<CoreMsg>) -> usize {
    tokio::task::block_in_place(|| {
        let (_sender, foreground) = flume::unbounded();
        let mut processed = 0;
        for _ in 0..128 {
            let Some(batch) = crate::try_recv_core_batch(&foreground, incoming) else {
                break;
            };
            processed += batch.len();
            assert!(crate::handle_core_batch_responsive(core, batch));
        }
        processed
    })
}

async fn sample_with_core(
    core: &mut AppCore,
    incoming: &flume::Receiver<CoreMsg>,
    duration: Duration,
) -> usize {
    let deadline = Instant::now() + duration;
    let mut processed = 0;
    loop {
        processed += dispatch_pending(core, incoming);
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        sleep(remaining.min(Duration::from_millis(20))).await;
    }
    processed
}

fn enqueue_discovery_replay(core: &mut AppCore, events: &[Event]) {
    for index in 0..MAX_SEEN_EVENT_IDS {
        core.remember_event(format!("{index:064x}"));
    }
    // Message traffic also replaces cached signature checks. Replaying the
    // genuine signed discovery events must pay authentication in this gate.
    core.event_validation = Default::default();
    for event in events {
        assert!(!core.has_seen_event(&event.id.to_hex()));
        core.core_sender
            .send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
                event.clone(),
            ))))
            .unwrap();
    }
}
