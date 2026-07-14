use super::*;
use fips_core::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config as TcpConfig, ConnectionId, State};
use fips_tcp_endpoint::{AdapterError, FipsTcpEndpoint};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant as StdInstant;
use tokio::task::JoinHandle;

use control::{enqueue_control_retries, enqueue_dirty_resyncs, mark_dirty, store_control_record};
use framing::{comparison_key, elapsed_millis, frame, random_isn_seed, RecordReader};

mod control;
mod framing;

const POLL_MILLIS: u64 = 25;
const PEER_REFRESH_MILLIS: u64 = 250;
const FRAME_HEADER_BYTES: usize = 4;
const COMMAND_CAPACITY: usize = 16;
const MAX_COMMAND_BATCH_RECORDS: usize = 64;
const MAX_COMMAND_BATCH_BYTES: usize = 3 * 1024 * 1024;
const MAX_TCP_CONNECTIONS: usize = 64;
const MAX_PENDING_RECORDS_PER_PEER: usize = 64;
const MAX_PENDING_RECORDS_TOTAL: usize = MAX_PENDING_RECORDS_PER_PEER * MAX_TCP_CONNECTIONS;
const MAX_DEFERRED_RECORDS_PER_PEER: usize = 256;
const MAX_DEFERRED_RECORDS_TOTAL: usize = 512;

#[derive(Clone)]
pub(super) struct DeviceSyncTcpSender {
    tx: flume::Sender<SendBatch>,
    max_record_bytes: usize,
    dirty: Arc<Mutex<HashSet<String>>>,
    control_retry: Arc<Mutex<HashMap<String, ControlRecord>>>,
}

pub(super) struct SendBatch {
    pub(super) peer: PeerIdentity,
    pub(super) records: VecDeque<Vec<u8>>,
}

struct PendingRecord {
    bytes: Vec<u8>,
    offset: usize,
    control: bool,
}

#[derive(Clone)]
struct ControlRecord {
    bytes: Vec<u8>,
    rank: Option<(u8, u64, String, String)>,
}

impl DeviceSyncTcpSender {
    pub(super) fn send_batch(&self, peer: PeerIdentity, records: Vec<Vec<u8>>) -> bool {
        if records
            .iter()
            .any(|record| record.len() > self.max_record_bytes)
            || records.len() > MAX_COMMAND_BATCH_RECORDS
            || records
                .iter()
                .try_fold(0_usize, |total, record| total.checked_add(record.len()))
                .is_none_or(|total| total > MAX_COMMAND_BATCH_BYTES)
        {
            return false;
        }
        let command = SendBatch {
            peer,
            records: records.into(),
        };
        if self.tx.try_send(command).is_ok() {
            return true;
        }
        mark_dirty(&self.dirty, peer.npub());
        false
    }

    pub(super) fn send_control(
        &self,
        peer: PeerIdentity,
        record: Vec<u8>,
        rank: Option<(u8, u64, String, String)>,
    ) -> bool {
        if record.len() > self.max_record_bytes {
            return false;
        }
        let Ok(mut records) = self.control_retry.lock() else {
            return false;
        };
        store_control_record(
            &mut records,
            peer.npub(),
            ControlRecord {
                bytes: record,
                rank,
            },
        );
        true
    }

    #[cfg(test)]
    pub(super) fn test_channel(
        capacity: usize,
        max_record_bytes: usize,
    ) -> (Self, flume::Receiver<SendBatch>) {
        let (tx, rx) = flume::bounded(capacity);
        (
            Self {
                tx,
                max_record_bytes,
                dirty: Arc::new(Mutex::new(HashSet::new())),
                control_retry: Arc::new(Mutex::new(HashMap::new())),
            },
            rx,
        )
    }

    #[cfg(test)]
    pub(super) fn take_control_for_test(&self, peer: PeerIdentity) -> Option<Vec<u8>> {
        self.control_retry
            .lock()
            .ok()
            .and_then(|mut records| records.remove(&peer.npub()).map(|record| record.bytes))
    }
}

pub(super) async fn start_device_sync_tcp(
    endpoint: Arc<FipsEndpoint>,
    service_port: u16,
    max_record_bytes: usize,
    initial_request: Vec<u8>,
    resync_required: Vec<u8>,
    core_sender: Sender<CoreMsg>,
) -> Result<(DeviceSyncTcpSender, JoinHandle<()>), String> {
    let config = TcpConfig {
        max_connections: MAX_TCP_CONNECTIONS,
        ..TcpConfig::default()
    };
    let tcp = FipsTcpEndpoint::bind(endpoint.clone(), service_port, config, random_isn_seed())
        .await
        .map_err(|error| error.to_string())?;
    let (tx, rx) = flume::bounded(COMMAND_CAPACITY);
    let dirty = Arc::new(Mutex::new(HashSet::new()));
    let control_retry = Arc::new(Mutex::new(HashMap::new()));
    let task = tokio::spawn(run_device_sync_tcp(
        endpoint,
        tcp,
        rx,
        dirty.clone(),
        control_retry.clone(),
        service_port,
        max_record_bytes,
        initial_request,
        resync_required,
        core_sender,
    ));
    Ok((
        DeviceSyncTcpSender {
            tx,
            max_record_bytes,
            dirty,
            control_retry,
        },
        task,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_device_sync_tcp(
    endpoint: Arc<FipsEndpoint>,
    mut tcp: FipsTcpEndpoint,
    commands: flume::Receiver<SendBatch>,
    dirty: Arc<Mutex<HashSet<String>>>,
    control_retry: Arc<Mutex<HashMap<String, ControlRecord>>>,
    service_port: u16,
    max_record_bytes: usize,
    initial_request: Vec<u8>,
    resync_required: Vec<u8>,
    core_sender: Sender<CoreMsg>,
) {
    let started = StdInstant::now();
    let local = match PeerIdentity::from_npub(endpoint.npub()) {
        Ok(identity) => identity,
        Err(_) => return,
    };
    let mut connected = HashSet::new();
    let mut connections = HashMap::<String, ConnectionId>::new();
    let mut readers = HashMap::<ConnectionId, RecordReader>::new();
    let mut pending = HashMap::<String, VecDeque<PendingRecord>>::new();
    let mut pending_count = 0;
    let mut requested = HashSet::<ConnectionId>::new();
    let mut last_peer_refresh = 0;
    let mut deferred = HashMap::<String, VecDeque<Vec<u8>>>::new();
    let mut deferred_count = 0;

    loop {
        enqueue_dirty_resyncs(&dirty, &mut pending, &mut pending_count, &resync_required);
        enqueue_control_retries(&control_retry, &mut pending, &mut pending_count);
        enqueue_deferred_records(
            &mut deferred,
            &mut deferred_count,
            &mut pending,
            &mut pending_count,
        );
        let now = elapsed_millis(started);
        tokio::select! {
            command = commands.recv_async() => {
                if let Ok(command) = command {
                    defer_batch(
                        command,
                        &dirty,
                        &mut deferred,
                        &mut deferred_count,
                    );
                }
            }
            received = tokio::time::timeout(
                Duration::from_millis(POLL_MILLIS),
                tcp.receive(now),
            ) => {
                if matches!(received, Ok(Err(AdapterError::Closed))) {
                    break;
                }
            }
        }

        let now = elapsed_millis(started);
        if now.saturating_sub(last_peer_refresh) >= PEER_REFRESH_MILLIS {
            last_peer_refresh = now;
            if let Ok(peers) = endpoint.peers().await {
                connected = peers
                    .into_iter()
                    .filter(|peer| peer.connected)
                    .map(|peer| peer.npub)
                    .collect();
                reconcile_connections(
                    &local,
                    &connected,
                    &mut connections,
                    &mut readers,
                    &mut pending,
                    &mut requested,
                    &mut tcp,
                    max_record_bytes,
                    now,
                )
                .await;
            }
        }

        let _ = tcp.poll(now).await;
        accept_connections(
            &connected,
            &mut connections,
            &mut readers,
            &mut pending,
            &mut requested,
            &mut tcp,
            max_record_bytes,
            now,
        )
        .await;
        progress_connections(
            &mut connections,
            &mut readers,
            &mut pending,
            &mut pending_count,
            &mut requested,
            &initial_request,
            &core_sender,
            service_port,
            &mut tcp,
            now,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn reconcile_connections(
    local: &PeerIdentity,
    connected: &HashSet<String>,
    connections: &mut HashMap<String, ConnectionId>,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    requested: &mut HashSet<ConnectionId>,
    tcp: &mut FipsTcpEndpoint,
    max_record_bytes: usize,
    now: u64,
) {
    let removed = connections
        .keys()
        .filter(|peer| !connected.contains(*peer))
        .cloned()
        .collect::<Vec<_>>();
    for peer in removed {
        if let Some(id) = remove_connection_state(&peer, connections, readers, pending, requested) {
            let _ = tcp.close(id, now).await;
        }
    }

    for npub in connected {
        if connections.contains_key(npub) {
            continue;
        }
        let Ok(peer) = PeerIdentity::from_npub(npub) else {
            continue;
        };
        if comparison_key(local) >= comparison_key(&peer) {
            continue;
        }
        if let Ok(id) = tcp.connect(peer, now).await {
            connections.insert(npub.clone(), id);
            readers.insert(id, RecordReader::new(max_record_bytes));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn accept_connections(
    connected: &HashSet<String>,
    connections: &mut HashMap<String, ConnectionId>,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    requested: &mut HashSet<ConnectionId>,
    tcp: &mut FipsTcpEndpoint,
    max_record_bytes: usize,
    now: u64,
) {
    while let Some(id) = tcp.accept() {
        let Some(identity) = tcp.peer(id) else {
            let _ = tcp.close(id, now).await;
            continue;
        };
        let peer = identity.npub();
        if !connected.contains(&peer) {
            let _ = tcp.close(id, now).await;
            continue;
        }
        if let Some(previous) =
            remove_connection_state(&peer, connections, readers, pending, requested)
        {
            let _ = tcp.close(previous, now).await;
        }
        connections.insert(peer, id);
        readers.insert(id, RecordReader::new(max_record_bytes));
    }
}

#[allow(clippy::too_many_arguments)]
async fn progress_connections(
    connections: &mut HashMap<String, ConnectionId>,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    pending_count: &mut usize,
    requested: &mut HashSet<ConnectionId>,
    initial_request: &[u8],
    core_sender: &Sender<CoreMsg>,
    service_port: u16,
    tcp: &mut FipsTcpEndpoint,
    now: u64,
) {
    for (peer, id) in connections.clone() {
        let Some(state) = tcp.state(id) else {
            remove_connection_state(&peer, connections, readers, pending, requested);
            continue;
        };
        if state != State::Established {
            continue;
        }
        if !requested.contains(&id)
            && enqueue_pending(
                pending,
                pending_count,
                peer.clone(),
                frame(initial_request),
                false,
                true,
            )
        {
            requested.insert(id);
        }
        drain_writes(&peer, id, pending, pending_count, tcp, now).await;
        drain_reads(&peer, id, readers, core_sender, service_port, tcp, now).await;
    }
}

async fn drain_writes(
    peer: &str,
    id: ConnectionId,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    pending_count: &mut usize,
    tcp: &mut FipsTcpEndpoint,
    now: u64,
) {
    let Some(records) = pending.get_mut(peer) else {
        return;
    };
    while let Some(record) = records.front_mut() {
        let Some(remaining) = record.bytes.get(record.offset..) else {
            records.pop_front();
            *pending_count = pending_count.saturating_sub(1);
            continue;
        };
        let Ok(accepted) = tcp.write(id, remaining, now).await else {
            break;
        };
        if accepted == 0 {
            break;
        }
        record.offset += accepted;
        if record.offset < record.bytes.len() {
            break;
        }
        records.pop_front();
        *pending_count = pending_count.saturating_sub(1);
    }
    if records.is_empty() {
        pending.remove(peer);
    }
}

fn enqueue_pending(
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    pending_count: &mut usize,
    peer: String,
    bytes: Vec<u8>,
    priority: bool,
    control: bool,
) -> bool {
    let records = pending.entry(peer).or_default();
    if priority {
        while (records.len() >= MAX_PENDING_RECORDS_PER_PEER
            || *pending_count >= MAX_PENDING_RECORDS_TOTAL)
            && records
                .back()
                .is_some_and(|record| record.offset == 0 && !record.control)
        {
            records.pop_back();
            *pending_count = pending_count.saturating_sub(1);
        }
    }
    if records.len() >= MAX_PENDING_RECORDS_PER_PEER || *pending_count >= MAX_PENDING_RECORDS_TOTAL
    {
        return false;
    }
    let record = PendingRecord {
        bytes,
        offset: 0,
        control,
    };
    if priority {
        let after_partial = usize::from(records.front().is_some_and(|record| record.offset > 0));
        records.insert(after_partial, record);
    } else {
        records.push_back(record);
    }
    *pending_count += 1;
    true
}

fn defer_batch(
    batch: SendBatch,
    dirty: &Arc<Mutex<HashSet<String>>>,
    deferred: &mut HashMap<String, VecDeque<Vec<u8>>>,
    deferred_count: &mut usize,
) {
    let peer = batch.peer.npub();
    let records = deferred.entry(peer.clone()).or_default();
    for record in batch.records {
        if records.len() >= MAX_DEFERRED_RECORDS_PER_PEER
            || *deferred_count >= MAX_DEFERRED_RECORDS_TOTAL
        {
            mark_dirty(dirty, peer.clone());
            break;
        }
        records.push_back(record);
        *deferred_count += 1;
    }
}

fn enqueue_deferred_records(
    deferred: &mut HashMap<String, VecDeque<Vec<u8>>>,
    deferred_count: &mut usize,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    pending_count: &mut usize,
) {
    for peer in deferred.keys().cloned().collect::<Vec<_>>() {
        let Some(records) = deferred.get_mut(&peer) else {
            continue;
        };
        while let Some(record) = records.front() {
            if !enqueue_pending(
                pending,
                pending_count,
                peer.clone(),
                frame(record),
                false,
                false,
            ) {
                break;
            }
            records.pop_front();
            *deferred_count = deferred_count.saturating_sub(1);
        }
        if records.is_empty() {
            deferred.remove(&peer);
        }
    }
}

fn rewind_pending(pending: &mut HashMap<String, VecDeque<PendingRecord>>, peer: &str) {
    if let Some(records) = pending.get_mut(peer) {
        for record in records {
            record.offset = 0;
        }
    }
}

fn remove_connection_state(
    peer: &str,
    connections: &mut HashMap<String, ConnectionId>,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    requested: &mut HashSet<ConnectionId>,
) -> Option<ConnectionId> {
    let id = connections.remove(peer)?;
    readers.remove(&id);
    requested.remove(&id);
    rewind_pending(pending, peer);
    Some(id)
}

async fn drain_reads(
    peer: &str,
    id: ConnectionId,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    core_sender: &Sender<CoreMsg>,
    service_port: u16,
    tcp: &mut FipsTcpEndpoint,
    now: u64,
) {
    let Some(reader) = readers.get_mut(&id) else {
        return;
    };
    loop {
        let Ok(bytes) = tcp.read(id, u16::MAX as usize, now).await else {
            return;
        };
        if bytes.is_empty() {
            return;
        }
        let Ok(records) = reader.push(&bytes) else {
            let _ = tcp.close(id, now).await;
            return;
        };
        let Ok(identity) = PeerIdentity::from_npub(peer) else {
            return;
        };
        for data in records {
            let _ = core_sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::DeviceSyncPacket {
                    source_pubkey_hex: identity.pubkey().to_string(),
                    source_port: service_port,
                    data,
                },
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn removing_stream_state_rewinds_partial_frame_and_prunes_request_id() {
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
        let Ok(accepted) = client.write(id, &record.bytes, 0) else {
            panic!("established stream should accept bytes");
        };
        record.offset += accepted;
        assert_eq!(record.offset, 8);

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
            .is_some_and(|record| record.offset == 0));
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

    #[tokio::test]
    async fn record_queued_while_disconnected_is_delivered_after_connect() {
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .without_system_tun()
                .bind()
                .await
                .expect("bind embedded endpoint"),
        );
        let local = PeerIdentity::from_npub(endpoint.npub()).expect("local identity");
        let peer = local.npub();
        let mut tcp =
            FipsTcpEndpoint::bind(endpoint.clone(), 39_017, TcpConfig::default(), 0x1234_5678)
                .await
                .expect("bind TCP service");
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

        let client = tcp.connect(local, 0).await.expect("connect after queue");
        for _ in 0..3 {
            tokio::time::timeout(Duration::from_secs(2), tcp.receive(0))
                .await
                .expect("handshake timed out")
                .expect("receive handshake");
        }
        let server = tcp.accept().expect("accept loopback connection");
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

        let bytes = tcp.read(server, 1024, 10).await.expect("read stream");
        let records = RecordReader::new(1024)
            .push(&bytes)
            .expect("parse framed record");
        assert_eq!(records, vec![payload.to_vec()]);
        assert_eq!(pending_count, 0);
        assert!(pending.is_empty());
        endpoint.shutdown().await.expect("shutdown endpoint");
    }
}
