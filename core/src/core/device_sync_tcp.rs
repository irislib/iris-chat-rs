use super::*;
use fips_core::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config as TcpConfig, ConnectionId, MarkerStatus, SendMarker, State};
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
    marker: Option<SendMarker>,
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
    allowed_peers: HashSet<String>,
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
        allowed_peers,
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
    allowed_peers: HashSet<String>,
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
                    .filter(|peer| peer.connected && allowed_peers.contains(&peer.npub))
                    .map(|peer| peer.npub)
                    .collect();
                reconcile_connections(
                    &local,
                    &connected,
                    &mut connections,
                    &mut readers,
                    &mut pending,
                    &mut requested,
                    &dirty,
                    &mut tcp,
                    max_record_bytes,
                    now,
                )
                .await;
            }
        }

        let _ = tcp.poll(now).await;
        progress_connections(
            &mut connections,
            &mut readers,
            &mut pending,
            &mut pending_count,
            &mut requested,
            &dirty,
            &initial_request,
            &core_sender,
            service_port,
            &mut tcp,
            now,
        )
        .await;
        accept_connections(
            &local,
            &allowed_peers,
            &connected,
            &mut connections,
            &mut readers,
            &mut pending,
            &mut requested,
            &dirty,
            &mut tcp,
            max_record_bytes,
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
    dirty: &Arc<Mutex<HashSet<String>>>,
    tcp: &mut FipsTcpEndpoint,
    max_record_bytes: usize,
    now: u64,
) {
    let stale = connections
        .iter()
        .filter(|&(peer, id)| {
            connection_requires_retirement(connected.contains(peer), tcp.state(*id))
        })
        .map(|(peer, id)| (peer.clone(), *id))
        .collect::<Vec<_>>();
    for (peer, id) in stale {
        retire_connection(
            &peer,
            id,
            connections,
            readers,
            pending,
            requested,
            dirty,
            tcp,
        )
        .await;
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
    local: &PeerIdentity,
    allowed_peers: &HashSet<String>,
    connected: &HashSet<String>,
    connections: &mut HashMap<String, ConnectionId>,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    requested: &mut HashSet<ConnectionId>,
    dirty: &Arc<Mutex<HashSet<String>>>,
    tcp: &mut FipsTcpEndpoint,
    max_record_bytes: usize,
) {
    while let Some(id) = tcp.accept() {
        let Some(identity) = tcp.peer(id) else {
            let _ = tcp.abort(id).await;
            continue;
        };
        let peer = identity.npub();
        if !allowed_peers.contains(&peer) || !connected.contains(&peer) {
            let _ = tcp.abort(id).await;
            continue;
        }
        if let Some(previous) = connections.get(&peer).copied() {
            let keep_previous = matches!(tcp.state(previous), Some(State::Established))
                || (comparison_key(local) < comparison_key(&identity)
                    && matches!(tcp.state(previous), Some(State::SynSent)));
            if keep_previous {
                let _ = tcp.abort(id).await;
                continue;
            }
            retire_connection(
                &peer,
                previous,
                connections,
                readers,
                pending,
                requested,
                dirty,
                tcp,
            )
            .await;
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
    dirty: &Arc<Mutex<HashSet<String>>>,
    initial_request: &[u8],
    core_sender: &Sender<CoreMsg>,
    service_port: u16,
    tcp: &mut FipsTcpEndpoint,
    now: u64,
) {
    for (peer, id) in connections.clone() {
        let Some(state) = tcp.state(id) else {
            retire_connection(
                &peer,
                id,
                connections,
                readers,
                pending,
                requested,
                dirty,
                tcp,
            )
            .await;
            continue;
        };
        match state {
            State::Established => {
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
            State::CloseWait => {
                drain_reads(&peer, id, readers, core_sender, service_port, tcp, now).await;
                retire_connection(
                    &peer,
                    id,
                    connections,
                    readers,
                    pending,
                    requested,
                    dirty,
                    tcp,
                )
                .await;
            }
            State::SynSent | State::SynReceived => {}
            State::FinWait1
            | State::FinWait2
            | State::Closing
            | State::LastAck
            | State::TimeWait => {
                retire_connection(
                    &peer,
                    id,
                    connections,
                    readers,
                    pending,
                    requested,
                    dirty,
                    tcp,
                )
                .await;
            }
        }
    }
}

fn connection_requires_retirement(connected: bool, state: Option<State>) -> bool {
    !connected
        || matches!(
            state,
            None | Some(
                State::FinWait1
                    | State::FinWait2
                    | State::Closing
                    | State::LastAck
                    | State::TimeWait
            )
        )
}

#[allow(clippy::too_many_arguments)]
async fn retire_connection(
    peer: &str,
    id: ConnectionId,
    connections: &mut HashMap<String, ConnectionId>,
    readers: &mut HashMap<ConnectionId, RecordReader>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    requested: &mut HashSet<ConnectionId>,
    dirty: &Arc<Mutex<HashSet<String>>>,
    tcp: &mut FipsTcpEndpoint,
) {
    mark_dirty(dirty, peer.to_string());
    if remove_connection_state(peer, connections, readers, pending, requested) == Some(id)
        && tcp.state(id).is_some()
    {
        let _ = tcp.abort(id).await;
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
        if let Some(marker) = record.marker {
            match tcp.marker_status(&marker) {
                MarkerStatus::Acked if record.offset == record.bytes.len() => {
                    records.pop_front();
                    *pending_count = pending_count.saturating_sub(1);
                    continue;
                }
                MarkerStatus::ConnectionGone => {
                    record.offset = 0;
                    record.marker = None;
                    break;
                }
                MarkerStatus::Pending | MarkerStatus::Acked => {}
            }
        }
        let Some(remaining) = record.bytes.get(record.offset..) else {
            records.pop_front();
            *pending_count = pending_count.saturating_sub(1);
            continue;
        };
        if remaining.is_empty() {
            break;
        }
        let Ok((accepted, marker)) = tcp.write_with_marker(id, remaining, now).await else {
            break;
        };
        if accepted == 0 {
            break;
        }
        record.offset += accepted;
        record.marker = Some(marker);
        if record.offset < record.bytes.len() {
            break;
        }
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
        marker: None,
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
            record.marker = None;
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
#[path = "device_sync_tcp/tests.rs"]
mod tests;
