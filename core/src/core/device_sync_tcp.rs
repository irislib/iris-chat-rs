use super::*;
use fips_core::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config as TcpConfig, ConnectionId, State};
use fips_tcp_endpoint::{AdapterError, FipsTcpEndpoint};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant as StdInstant;
use tokio::task::JoinHandle;

const POLL_MILLIS: u64 = 25;
const PEER_REFRESH_MILLIS: u64 = 250;
const FRAME_HEADER_BYTES: usize = 4;
const COMMAND_CAPACITY: usize = 256;
const MAX_PENDING_RECORDS_PER_PEER: usize = 64;
const MAX_PENDING_RECORDS_TOTAL: usize = 256;

#[derive(Clone)]
pub(super) struct DeviceSyncTcpSender {
    tx: flume::Sender<SendRecord>,
    max_record_bytes: usize,
}

struct SendRecord {
    peer: PeerIdentity,
    record: Vec<u8>,
}

struct PendingRecord {
    bytes: Vec<u8>,
    offset: usize,
}

impl DeviceSyncTcpSender {
    pub(super) fn send(&self, peer: PeerIdentity, record: Vec<u8>) -> bool {
        if record.len() > self.max_record_bytes {
            return false;
        }
        self.tx.try_send(SendRecord { peer, record }).is_ok()
    }
}

pub(super) async fn start_device_sync_tcp(
    endpoint: Arc<FipsEndpoint>,
    service_port: u16,
    max_record_bytes: usize,
    initial_request: Vec<u8>,
    core_sender: Sender<CoreMsg>,
) -> Result<(DeviceSyncTcpSender, JoinHandle<()>), String> {
    let config = TcpConfig {
        max_connections: 64,
        ..TcpConfig::default()
    };
    let tcp = FipsTcpEndpoint::bind(endpoint.clone(), service_port, config, random_isn_seed())
        .await
        .map_err(|error| error.to_string())?;
    let (tx, rx) = flume::bounded(COMMAND_CAPACITY);
    let task = tokio::spawn(run_device_sync_tcp(
        endpoint,
        tcp,
        rx,
        service_port,
        max_record_bytes,
        initial_request,
        core_sender,
    ));
    Ok((
        DeviceSyncTcpSender {
            tx,
            max_record_bytes,
        },
        task,
    ))
}

async fn run_device_sync_tcp(
    endpoint: Arc<FipsEndpoint>,
    mut tcp: FipsTcpEndpoint,
    commands: flume::Receiver<SendRecord>,
    service_port: u16,
    max_record_bytes: usize,
    initial_request: Vec<u8>,
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

    loop {
        let now = elapsed_millis(started);
        tokio::select! {
            command = commands.recv_async() => {
                if let Ok(command) = command {
                    if command.record.len() > max_record_bytes {
                        continue;
                    }
                    enqueue_pending(
                        &mut pending,
                        &mut pending_count,
                        command.peer.npub(),
                        frame(&command.record),
                        false,
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
        if requested.insert(id) {
            enqueue_pending(
                pending,
                pending_count,
                peer.clone(),
                frame(initial_request),
                true,
            );
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
) {
    let records = pending.entry(peer).or_default();
    if priority {
        while (records.len() >= MAX_PENDING_RECORDS_PER_PEER
            || *pending_count >= MAX_PENDING_RECORDS_TOTAL)
            && records.back().is_some_and(|record| record.offset == 0)
        {
            records.pop_back();
            *pending_count = pending_count.saturating_sub(1);
        }
    }
    if records.len() >= MAX_PENDING_RECORDS_PER_PEER || *pending_count >= MAX_PENDING_RECORDS_TOTAL
    {
        return;
    }
    let record = PendingRecord { bytes, offset: 0 };
    if priority {
        records.push_front(record);
    } else {
        records.push_back(record);
    }
    *pending_count += 1;
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

struct RecordReader {
    bytes: Vec<u8>,
    max_record_bytes: usize,
}

impl RecordReader {
    fn new(max_record_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_record_bytes,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
        self.bytes.extend_from_slice(chunk);
        let mut records = Vec::new();
        let mut consumed = 0;
        while self.bytes.len().saturating_sub(consumed) >= FRAME_HEADER_BYTES {
            let header = self
                .bytes
                .get(consumed..consumed + FRAME_HEADER_BYTES)
                .ok_or(())?;
            let length = u32::from_be_bytes(header.try_into().map_err(|_| ())?) as usize;
            if length > self.max_record_bytes {
                return Err(());
            }
            let end = consumed + FRAME_HEADER_BYTES + length;
            if end > self.bytes.len() {
                break;
            }
            records.push(
                self.bytes
                    .get(consumed + FRAME_HEADER_BYTES..end)
                    .ok_or(())?
                    .to_vec(),
            );
            consumed = end;
        }
        self.bytes.drain(..consumed);
        Ok(records)
    }
}

fn frame(record: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FRAME_HEADER_BYTES + record.len());
    bytes.extend_from_slice(&(record.len() as u32).to_be_bytes());
    bytes.extend_from_slice(record);
    bytes
}

fn comparison_key(identity: &PeerIdentity) -> String {
    let value = identity.pubkey().to_string().to_lowercase();
    if value.len() == 66 && (value.starts_with("02") || value.starts_with("03")) {
        value[2..].to_owned()
    } else {
        value
    }
}

fn elapsed_millis(started: StdInstant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn random_isn_seed() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs() ^ u64::from(now.subsec_nanos())
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
    fn pending_records_are_bounded_and_reconnect_request_has_priority() {
        let mut pending = HashMap::new();
        let mut count = 0;
        let peer = "peer".to_owned();
        for value in 0..MAX_PENDING_RECORDS_PER_PEER {
            enqueue_pending(
                &mut pending,
                &mut count,
                peer.clone(),
                frame(&[value as u8]),
                false,
            );
        }
        enqueue_pending(
            &mut pending,
            &mut count,
            peer.clone(),
            frame(b"dropped"),
            false,
        );
        assert_eq!(pending[&peer].len(), MAX_PENDING_RECORDS_PER_PEER);
        assert_eq!(count, MAX_PENDING_RECORDS_PER_PEER);

        let request = frame(b"request");
        enqueue_pending(
            &mut pending,
            &mut count,
            peer.clone(),
            request.clone(),
            true,
        );
        assert_eq!(pending[&peer].len(), MAX_PENDING_RECORDS_PER_PEER);
        assert_eq!(pending[&peer].front().unwrap().bytes, request);
        assert_eq!(count, MAX_PENDING_RECORDS_PER_PEER);
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
        enqueue_pending(
            &mut pending,
            &mut count,
            peer.clone(),
            frame(b"long-enough-to-be-partial"),
            false,
        );
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
        };
        let identity = fips_core::Identity::generate();
        let peer = PeerIdentity::from_pubkey_full(identity.pubkey_full());
        assert!(!sender.send(peer, vec![0; 5]));
        assert!(rx.is_empty());
    }
}
