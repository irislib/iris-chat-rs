//! Recipient-approved file streams. Only metadata and progress leave this task;
//! file bytes never enter chat events or the attachment/blob storage machinery.
use fips_core::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config, ConnectionId, MarkerStatus, SendMarker, State};
use fips_tcp_endpoint::FipsTcpEndpoint;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Instant,
};
use tokio::task::JoinHandle;

mod files;
mod wire;
use files::{ReceiveFiles, SendFiles};
use wire::{Control, Reader};

const PORT: u16 = 39512;
const MAX_TRANSFERS: usize = 16;
const IDLE_MS: u64 = 60_000;
const OFFER_MS: u64 = 24 * 60 * 60 * 1000;
const CHUNK: usize = 32 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct TransferFile {
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) enum DirectFileEvent {
    Accepted {
        transfer_id: String,
        peer: String,
    },
    Progress {
        transfer_id: String,
        peer: String,
        transferred_bytes: u64,
        total_bytes: u64,
    },
    Completed {
        transfer_id: String,
        peer: String,
        local_paths: Vec<String>,
    },
    Declined {
        transfer_id: String,
        peer: String,
    },
    Cancelled {
        transfer_id: String,
        peer: String,
    },
    Failed {
        transfer_id: String,
        peer: String,
        error: String,
    },
}

#[derive(Clone)]
pub(crate) struct DirectFileSender(flume::Sender<Command>, Restrictions);
type Restrictions = Arc<RwLock<HashMap<String, HashSet<String>>>>;
enum Command {
    Offer {
        id: String,
        token: String,
        peers: Vec<PeerIdentity>,
        files: Vec<TransferFile>,
    },
    Receive {
        id: String,
        token: String,
        peer: PeerIdentity,
        files: Vec<TransferFile>,
        destination: Arc<dyn crate::DirectFileDestination>,
    },
    Decline {
        id: String,
        token: String,
        peer: PeerIdentity,
    },
    Cancel(String),
}
impl DirectFileSender {
    /// Apply current roster authorization immediately, scoped to one capability.
    /// This can only narrow the offer's original recipient list.
    pub fn restrict_offer(&self, id: &str, peers: Vec<PeerIdentity>) -> Result<(), String> {
        self.1
            .write()
            .map_err(|_| "File authorization unavailable")?
            .insert(id.into(), peers.into_iter().map(peer_hex).collect());
        Ok(())
    }
    pub fn register_offer(
        &self,
        id: String,
        token: String,
        allowed_peers: Vec<PeerIdentity>,
        files: Vec<TransferFile>,
    ) -> Result<(), String> {
        wire::validate_claim(&id, &token)?;
        files::validate(&files)?;
        if allowed_peers.is_empty() || allowed_peers.len() > 128 {
            return Err("No receiving device is available".into());
        }
        self.send(Command::Offer {
            id,
            token,
            peers: allowed_peers,
            files,
        })
    }
    pub fn receive(
        &self,
        id: String,
        token: String,
        sender_peer: PeerIdentity,
        files: Vec<TransferFile>,
        destination_dir: PathBuf,
    ) -> Result<(), String> {
        // The smoke/transport harness creates its own isolated destination.
        std::fs::create_dir_all(&destination_dir).map_err(|e| e.to_string())?;
        self.receive_into(
            id,
            token,
            sender_peer,
            files,
            crate::direct_file_directory_destination(
                destination_dir.to_string_lossy().into_owned(),
            ),
        )
    }
    pub fn receive_into(
        &self,
        id: String,
        token: String,
        sender_peer: PeerIdentity,
        files: Vec<TransferFile>,
        destination: Arc<dyn crate::DirectFileDestination>,
    ) -> Result<(), String> {
        wire::validate_claim(&id, &token)?;
        files::validate(&files)?;
        self.send(Command::Receive {
            id,
            token,
            peer: sender_peer,
            files,
            destination,
        })
    }
    pub fn decline(
        &self,
        id: String,
        token: String,
        sender_peer: PeerIdentity,
    ) -> Result<(), String> {
        wire::validate_claim(&id, &token)?;
        self.send(Command::Decline {
            id,
            token,
            peer: sender_peer,
        })
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        self.send(Command::Cancel(id.into()))
    }
    fn send(&self, command: Command) -> Result<(), String> {
        self.0
            .try_send(command)
            .map_err(|_| "File transfer is busy. Try again.".into())
    }
}

pub(crate) async fn start_direct_file_tcp(
    endpoint: Arc<FipsEndpoint>,
    event_tx: flume::Sender<DirectFileEvent>,
) -> Result<(DirectFileSender, JoinHandle<()>), String> {
    let tcp = FipsTcpEndpoint::bind(
        endpoint.clone(),
        PORT,
        Config {
            max_connections: MAX_TRANSFERS,
            max_connections_per_peer: 4,
            send_buffer: 64 * 1024,
            receive_buffer: u16::MAX as usize,
            ..Config::default()
        },
        rand::random(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let (tx, rx) = flume::bounded(32);
    let restrictions = Arc::new(RwLock::new(HashMap::new()));
    let local = PeerIdentity::from_npub(endpoint.npub()).map_err(|error| error.to_string())?;
    Ok((
        DirectFileSender(tx, restrictions.clone()),
        tokio::spawn(run(tcp, local, rx, event_tx, restrictions)),
    ))
}

struct Offer {
    token: String,
    peers: Vec<PeerIdentity>,
    files: Vec<TransferFile>,
    expires: u64,
}
struct Pending {
    bytes: Vec<u8>,
    offset: usize,
    marker: Option<SendMarker>,
}
impl Pending {
    fn control(value: Control) -> Result<Self, String> {
        Ok(Self::bytes(wire::control(value)?))
    }
    fn bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            offset: 0,
            marker: None,
        }
    }
}
enum Role {
    AwaitRequest,
    Sending(SendFiles),
    Receiving(ReceiveFiles),
    // Wait until a final acknowledgement has left the bounded TCP queue.
    Finish,
    Decline,
}
struct Connection {
    id: String,
    peer: PeerIdentity,
    role: Role,
    reader: Reader,
    pending: Option<Pending>,
    last_activity: u64,
    last_progress: u64,
    done_sent: bool,
}
impl Connection {
    fn new(id: String, peer: PeerIdentity, role: Role, now: u64) -> Self {
        Self {
            id,
            peer,
            role,
            reader: Reader::default(),
            pending: None,
            last_activity: now,
            last_progress: now,
            done_sent: false,
        }
    }
    fn peer_hex(&self) -> String {
        peer_hex(self.peer)
    }
    fn fail(self, events: &flume::Sender<DirectFileEvent>, error: impl Into<String>) {
        if !self.id.is_empty() && !matches!(self.role, Role::Finish | Role::Decline) {
            let event = DirectFileEvent::Failed {
                transfer_id: self.id.clone(),
                peer: self.peer_hex(),
                error: error.into(),
            };
            drop(self);
            let _ = events.send(event);
        }
    }
}
fn peer_hex(peer: PeerIdentity) -> String {
    let hex = peer.pubkey().to_string();
    if hex.len() == 66 {
        hex.get(2..).unwrap_or(&hex).to_owned()
    } else {
        hex
    }
}
fn now(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}
fn permitted(restrictions: &Restrictions, id: &str, peer: PeerIdentity) -> bool {
    restrictions.read().is_ok_and(|values| {
        values
            .get(id)
            .is_none_or(|peers| peers.contains(&peer_hex(peer)))
    })
}

async fn run(
    mut tcp: FipsTcpEndpoint,
    local: PeerIdentity,
    commands: flume::Receiver<Command>,
    events: flume::Sender<DirectFileEvent>,
    restrictions: Restrictions,
) {
    let started = Instant::now();
    let mut offers = HashMap::<String, Offer>::new();
    let mut connections = HashMap::<ConnectionId, Connection>::new();
    loop {
        let time = now(started);
        tokio::select! {
            command = commands.recv_async() => match command {
                Ok(command) => handle_command(command, local, &mut tcp, &mut offers, &mut connections, &events, &restrictions, time).await,
                Err(_) => break,
            },
            result = tokio::time::timeout(std::time::Duration::from_millis(10), tcp.receive(time)) => {
                if matches!(result, Ok(Err(fips_tcp_endpoint::AdapterError::Closed))) { break; }
            },
        }
        let time = now(started);
        // Read fresh policy each poll; roster changes cannot sit behind data or
        // command backlog. The offer's initial authenticated recipients remain
        // an additional independent constraint.
        let current = restrictions.read().map(|v| v.clone()).unwrap_or_default();
        offers.retain(|id, offer| {
            if let Some(peers) = current.get(id) {
                offer.peers.retain(|peer| peers.contains(&peer_hex(*peer)));
                if offer.peers.is_empty() {
                    let _ = events.send(DirectFileEvent::Failed {
                        transfer_id: id.clone(),
                        peer: String::new(),
                        error: "Receiving device authorization changed".into(),
                    });
                    if let Ok(mut values) = restrictions.write() {
                        values.remove(id);
                    }
                    return false;
                }
            }
            true
        });
        let _ = tcp.poll(time).await;
        while let Some(id) = tcp.accept() {
            let Some(peer) = tcp.peer(id) else {
                let _ = tcp.abort(id).await;
                continue;
            };
            if peer == local || connections.len() >= MAX_TRANSFERS {
                let _ = tcp.abort(id).await;
            } else {
                connections.insert(
                    id,
                    Connection::new(String::new(), peer, Role::AwaitRequest, time),
                );
            }
        }
        for id in connections.keys().copied().collect::<Vec<_>>() {
            let Some(mut connection) = connections.remove(&id) else {
                continue;
            };
            if current
                .get(&connection.id)
                .is_some_and(|peers| !peers.contains(&connection.peer_hex()))
            {
                let transfer_id = connection.id.clone();
                connection.fail(&events, "Device authorization changed");
                let _ = tcp.abort(id).await;
                if let Ok(mut values) = restrictions.write() {
                    values.remove(&transfer_id);
                }
                continue;
            }
            let result = progress(
                id,
                &mut connection,
                &mut tcp,
                &mut offers,
                &events,
                &restrictions,
                time,
            )
            .await;
            match result {
                Ok(true) => {
                    connections.insert(id, connection);
                }
                Ok(false) => {
                    if let Ok(mut values) = restrictions.write() {
                        values.remove(&connection.id);
                    }
                    let _ = tcp.close(id, time).await;
                }
                Err(error) => {
                    if let Ok(mut values) = restrictions.write() {
                        values.remove(&connection.id);
                    }
                    connection.fail(&events, error);
                    let _ = tcp.abort(id).await;
                }
            }
        }
        offers.retain(|id, offer| {
            if time < offer.expires {
                return true;
            }
            let _ = events.send(DirectFileEvent::Failed {
                transfer_id: id.clone(),
                peer: String::new(),
                error: "File offer expired".into(),
            });
            if let Ok(mut values) = restrictions.write() {
                values.remove(id);
            }
            false
        });
    }
    for (_, connection) in connections {
        connection.fail(&events, "File connection stopped");
    }
    for (id, _) in offers {
        let _ = events.send(DirectFileEvent::Failed {
            transfer_id: id,
            peer: String::new(),
            error: "File connection stopped".into(),
        });
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_command(
    command: Command,
    local: PeerIdentity,
    tcp: &mut FipsTcpEndpoint,
    offers: &mut HashMap<String, Offer>,
    connections: &mut HashMap<ConnectionId, Connection>,
    events: &flume::Sender<DirectFileEvent>,
    restrictions: &Restrictions,
    time: u64,
) {
    match command {
        Command::Offer {
            id,
            token,
            peers,
            files,
        } => {
            if offers.contains_key(&id)
                || connections.values().any(|c| c.id == id)
                || offers.len() >= MAX_TRANSFERS
            {
                let _ = events.send(DirectFileEvent::Failed {
                    transfer_id: id,
                    peer: String::new(),
                    error: "Too many file transfers".into(),
                });
            } else {
                offers.insert(
                    id,
                    Offer {
                        token,
                        peers: peers.into_iter().filter(|p| *p != local).collect(),
                        files,
                        expires: time + OFFER_MS,
                    },
                );
            }
        }
        Command::Receive {
            id,
            token,
            peer,
            files,
            destination,
        } => {
            if connections.values().any(|c| c.id == id) {
                return;
            }
            let result = async {
                if peer == local {
                    return Err("Choose another device to receive these files".to_string());
                }
                let receive = ReceiveFiles::new(files, destination, &id)?;
                let stream = tcp.connect(peer, time).await.map_err(|e| e.to_string())?;
                let mut connection =
                    Connection::new(id.clone(), peer, Role::Receiving(receive), time);
                connection.pending = Some(Pending::control(Control::Accept {
                    id: id.clone(),
                    token,
                })?);
                connections.insert(stream, connection);
                Ok::<_, String>(())
            }
            .await;
            if let Err(error) = result {
                let _ = events.send(DirectFileEvent::Failed {
                    transfer_id: id,
                    peer: peer_hex(peer),
                    error,
                });
            }
        }
        Command::Decline { id, token, peer } => {
            let pending = match Pending::control(Control::Decline {
                id: id.clone(),
                token,
            }) {
                Ok(pending) => pending,
                Err(error) => {
                    let _ = events.send(DirectFileEvent::Failed {
                        transfer_id: id,
                        peer: peer_hex(peer),
                        error,
                    });
                    return;
                }
            };
            if let Ok(stream) = tcp.connect(peer, time).await {
                let mut connection = Connection::new(id.clone(), peer, Role::Decline, time);
                connection.pending = Some(pending);
                connections.insert(stream, connection);
            }
        }
        Command::Cancel(id) => {
            offers.remove(&id);
            if let Ok(mut values) = restrictions.write() {
                values.remove(&id);
            }
            let mut reported = false;
            for connection in connections.values_mut().filter(|c| c.id == id) {
                // A partially emitted chunk must finish before the cancellation frame.
                // Its buffer is bounded; switching roles immediately closes disk handles.
                connection.role = Role::Finish;
                connection.done_sent = false;
                let _ = events.send(DirectFileEvent::Cancelled {
                    transfer_id: id.clone(),
                    peer: connection.peer_hex(),
                });
                reported = true;
            }
            if !reported {
                let _ = events.send(DirectFileEvent::Cancelled {
                    transfer_id: id,
                    peer: String::new(),
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn progress(
    id: ConnectionId,
    c: &mut Connection,
    tcp: &mut FipsTcpEndpoint,
    offers: &mut HashMap<String, Offer>,
    events: &flume::Sender<DirectFileEvent>,
    restrictions: &Restrictions,
    time: u64,
) -> Result<bool, String> {
    let state = tcp
        .state(id)
        .ok_or_else(|| "File connection closed".to_string())?;
    if time.saturating_sub(c.last_activity) > IDLE_MS {
        return Err("File transfer timed out".into());
    }
    if matches!(state, State::SynSent | State::SynReceived) {
        return Ok(true);
    }
    if !matches!(state, State::Established | State::CloseWait) {
        return Err("File connection closed".into());
    }
    // Read only one bounded batch per poll so no peer can monopolize the task.
    let bytes = tcp
        .read(id, CHUNK + 1024, time)
        .await
        .map_err(|e| e.to_string())?;
    if !bytes.is_empty() {
        c.last_activity = time;
        for packet in c.reader.push(&bytes)? {
            if !handle_packet(packet, c, offers, events, restrictions)? {
                return Ok(false);
            }
        }
    }
    if matches!(state, State::CloseWait) && !matches!(c.role, Role::Finish | Role::Decline) {
        return Err("File connection closed before completion".into());
    }
    if let Some(pending) = c.pending.as_mut() {
        if let Some(marker) = pending.marker {
            if tcp.marker_status(&marker) == MarkerStatus::Acked
                && pending.offset == pending.bytes.len()
            {
                c.pending = None;
                c.last_activity = time;
            }
        }
    }
    if c.pending.is_none() {
        match &mut c.role {
            Role::Sending(files) => {
                if let Some(data) = files.next_chunk()? {
                    c.pending = Some(Pending::bytes(wire::data(data)));
                } else if !c.done_sent {
                    c.pending = Some(Pending::control(Control::Done)?);
                    c.done_sent = true;
                }
            }
            Role::Finish if !c.done_sent => {
                c.pending = Some(Pending::control(Control::Cancel)?);
                c.done_sent = true;
            }
            Role::Finish => return Ok(false),
            Role::Decline => {
                let _ = events.send(DirectFileEvent::Declined {
                    transfer_id: c.id.clone(),
                    peer: c.peer_hex(),
                });
                return Ok(false);
            }
            _ => {}
        }
    }
    if let Some(pending) = c.pending.as_mut() {
        if pending.offset < pending.bytes.len() {
            if !permitted(restrictions, &c.id, c.peer) {
                return Err("Device authorization changed".into());
            }
            let remaining = pending
                .bytes
                .get(pending.offset..)
                .ok_or("Invalid file write offset")?;
            let (accepted, marker) = tcp
                .write_with_marker(id, remaining, time)
                .await
                .map_err(|e| e.to_string())?;
            if accepted > 0 {
                pending.offset += accepted;
                pending.marker = Some(marker);
                c.last_activity = time;
            }
        }
    }
    if time.saturating_sub(c.last_progress) >= 250 {
        let progress = match &c.role {
            Role::Sending(f) => Some((f.transferred(), f.total())),
            Role::Receiving(f) => Some((f.transferred(), f.total())),
            _ => None,
        };
        if let Some((transferred_bytes, total_bytes)) = progress {
            let _ = events.send(DirectFileEvent::Progress {
                transfer_id: c.id.clone(),
                peer: c.peer_hex(),
                transferred_bytes,
                total_bytes,
            });
        }
        c.last_progress = time;
    }
    Ok(true)
}

fn handle_packet(
    packet: Vec<u8>,
    c: &mut Connection,
    offers: &mut HashMap<String, Offer>,
    events: &flume::Sender<DirectFileEvent>,
    restrictions: &Restrictions,
) -> Result<bool, String> {
    if packet.first() == Some(&1) {
        return match &mut c.role {
            Role::Receiving(files) => {
                files.write(packet.get(1..).ok_or("Invalid file data frame")?)?;
                Ok(true)
            }
            Role::Finish => Ok(true),
            _ => Err("Unexpected file data".into()),
        };
    }
    let control = wire::parse_control(&packet)?;
    if matches!(c.role, Role::Finish) {
        return Ok(!matches!(control, Control::Cancel));
    }
    match control {
        Control::Accept { id, token } | Control::Decline { id, token }
            if matches!(c.role, Role::AwaitRequest) =>
        {
            wire::validate_claim(&id, &token)?;
            if !permitted(restrictions, &id, c.peer) {
                return Err("Device authorization changed".into());
            }
            let offer = offers.get(&id).ok_or("File offer is no longer available")?;
            if offer.token != token || !offer.peers.contains(&c.peer) {
                return Err("File offer is not available to this device".into());
            }
            let decline = matches!(wire::parse_control(&packet)?, Control::Decline { .. });
            // Removing the offer atomically claims it for this authenticated device.
            let offer = offers
                .remove(&id)
                .ok_or("File offer is no longer available")?;
            c.id = id.clone();
            if decline {
                let _ = events.send(DirectFileEvent::Declined {
                    transfer_id: id,
                    peer: c.peer_hex(),
                });
                return Ok(false);
            }
            c.role = Role::Sending(SendFiles::new(offer.files)?);
            let _ = events.send(DirectFileEvent::Accepted {
                transfer_id: id,
                peer: c.peer_hex(),
            });
        }
        Control::Done => {
            let Role::Receiving(files) = &mut c.role else {
                return Err("Unexpected file completion".into());
            };
            let paths = files.commit()?;
            let _ = events.send(DirectFileEvent::Completed {
                transfer_id: c.id.clone(),
                peer: c.peer_hex(),
                local_paths: paths,
            });
            c.role = Role::Finish;
            c.pending = Some(Pending::control(Control::Received)?);
            c.done_sent = true;
        }
        Control::Received if matches!(c.role, Role::Sending(_)) && c.done_sent => {
            let _ = events.send(DirectFileEvent::Completed {
                transfer_id: c.id.clone(),
                peer: c.peer_hex(),
                local_paths: Vec::new(),
            });
            return Ok(false);
        }
        Control::Cancel if !matches!(c.role, Role::AwaitRequest) => {
            c.role = Role::Finish;
            let _ = events.send(DirectFileEvent::Cancelled {
                transfer_id: c.id.clone(),
                peer: c.peer_hex(),
            });
            return Ok(false);
        }
        _ => return Err("Invalid file transfer request".into()),
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
