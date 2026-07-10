use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iris_chat_protocol::{
    decode_nearby_envelope_frame, encode_nearby_envelope_frame, NearbyEnvelope, NearbyInventoryItem,
};
use rand::RngCore;
use serde_json::Value;
use socket2::{Domain, Protocol, Socket, Type};

use crate::{DesktopNearbyObserver, DesktopNearbyPeerSnapshot, DesktopNearbySnapshot, FfiApp};

const SERVICE_TYPE: &str = "_iris-chat._tcp.local";
const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const NEARBY_HEADER_BYTES: usize = 13;
const MAX_FRAME_BODY_BYTES: usize = 256 * 1024;
const SINGLE_FRAME_BYTES: usize = 16 * 1024;
const HELLO_INTERVAL: Duration = Duration::from_secs(5);
const PRESENCE_RESEND_INTERVAL: Duration = Duration::from_secs(60);
const INVENTORY_RESEND_INTERVAL: Duration = Duration::from_secs(60);
const PEER_TTL: Duration = Duration::from_secs(15);
const MDNS_QUERY_INTERVAL: Duration = Duration::from_secs(5);
const MDNS_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(10);
const MAX_MAILBAG_EVENTS: usize = 500;
const NEARBY_PRESENCE_KIND: u32 = 22242;

pub struct DesktopNearbyService {
    app: Arc<FfiApp>,
    observer: Arc<dyn DesktopNearbyObserver>,
    inner: Arc<Mutex<DesktopNearbyInner>>,
}

struct DesktopNearbyInner {
    visible: bool,
    status: String,
    peer_id: String,
    local_nonce: String,
    local_name: String,
    own_profile_event_id: Option<String>,
    own_outbound: HashMap<String, StoredNearbyEvent>,
    forwarded: HashMap<String, StoredNearbyEvent>,
    known_profiles: HashMap<String, NearbyProfileEvent>,
    peers: HashMap<String, DesktopNearbyPeer>,
    peer_nonces: HashMap<String, String>,
    peer_inventory_sent_at: HashMap<String, Instant>,
    presence_sent_at: HashMap<String, Instant>,
    connection_nonces: HashMap<String, String>,
    connections: HashMap<String, DesktopNearbyConnection>,
    endpoint_keys: HashSet<String>,
    mdns_instances: HashMap<String, MdnsInstance>,
    stop_flag: Arc<AtomicBool>,
}

struct DesktopNearbyConnection {
    writer: Arc<Mutex<TcpStream>>,
    peer_id: Option<String>,
}

fn lock_desktop_nearby_inner(
    inner: &Arc<Mutex<DesktopNearbyInner>>,
) -> MutexGuard<'_, DesktopNearbyInner> {
    inner.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[derive(Clone)]
struct DesktopNearbyPeer {
    id: String,
    name: String,
    owner_pubkey_hex: Option<String>,
    picture_url: Option<String>,
    profile_event_id: Option<String>,
    last_seen: Instant,
}

#[derive(Clone)]
struct StoredNearbyEvent {
    id: String,
    kind: u32,
    created_at_secs: u64,
    event_json: String,
    author_pubkey_hex: Option<String>,
}

#[derive(Clone)]
struct NearbyProfileEvent {
    id: String,
    owner_pubkey_hex: String,
    display_name: Option<String>,
    picture_url: Option<String>,
}

#[derive(Default)]
struct MdnsInstance {
    target: Option<String>,
    port: Option<u16>,
    addr: Option<Ipv4Addr>,
    peer_id: Option<String>,
}

impl DesktopNearbyService {
    pub fn new(app: Arc<FfiApp>, observer: Arc<dyn DesktopNearbyObserver>) -> Arc<Self> {
        Arc::new(Self {
            app,
            observer,
            inner: Arc::new(Mutex::new(DesktopNearbyInner {
                visible: false,
                status: "Off".to_string(),
                peer_id: random_id(),
                local_nonce: random_id(),
                local_name: "Iris".to_string(),
                own_profile_event_id: None,
                own_outbound: HashMap::new(),
                forwarded: HashMap::new(),
                known_profiles: HashMap::new(),
                peers: HashMap::new(),
                peer_nonces: HashMap::new(),
                peer_inventory_sent_at: HashMap::new(),
                presence_sent_at: HashMap::new(),
                connection_nonces: HashMap::new(),
                connections: HashMap::new(),
                endpoint_keys: HashSet::new(),
                mdns_instances: HashMap::new(),
                stop_flag: Arc::new(AtomicBool::new(false)),
            })),
        })
    }

    pub fn start(&self, local_name: String) {
        let local_addr = match private_local_ipv4() {
            Some(addr) => addr,
            None => {
                self.set_status("Local network unavailable");
                return;
            }
        };
        let listener = match TcpListener::bind(SocketAddrV4::new(local_addr, 0)) {
            Ok(listener) => listener,
            Err(_) => {
                self.set_status("Local network unavailable");
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(_) => {
                self.set_status("Local network unavailable");
                return;
            }
        };
        let udp = match mdns_socket(local_addr) {
            Ok(socket) => socket,
            Err(_) => {
                self.set_status("Local network unavailable");
                return;
            }
        };

        let stop_flag = Arc::new(AtomicBool::new(false));
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if inner.visible {
                drop(inner);
                self.announce_to_connected_peers();
                return;
            }
            inner.visible = true;
            inner.status = "Starting".to_string();
            inner.local_nonce = random_id();
            inner.local_name = clean_name(&local_name);
            inner.stop_flag.store(true, Ordering::SeqCst);
            inner.stop_flag = stop_flag.clone();
            inner.connections.clear();
            inner.endpoint_keys.clear();
            inner.mdns_instances.clear();
        }
        self.notify();

        let _ = listener.set_nonblocking(true);
        let accept_self = self.clone_handles();
        thread::spawn(move || accept_self.accept_loop(listener, stop_flag));

        let mdns_self = self.clone_handles();
        let mdns_stop = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            inner.stop_flag.clone()
        };
        thread::spawn(move || mdns_self.mdns_loop(udp, local_addr, port, mdns_stop));

        self.set_status("Visible");
        self.announce_to_connected_peers();
    }

    pub fn stop(&self) {
        let writers = {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            inner.stop_flag.store(true, Ordering::SeqCst);
            inner.visible = false;
            inner.status = "Off".to_string();
            inner.peer_nonces.clear();
            inner.connection_nonces.clear();
            inner.peers.clear();
            inner.endpoint_keys.clear();
            inner.mdns_instances.clear();
            inner
                .connections
                .drain()
                .map(|(_, connection)| connection.writer)
                .collect::<Vec<_>>()
        };
        for writer in writers {
            if let Ok(stream) = writer.lock() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        self.notify();
    }

    pub fn snapshot(&self) -> DesktopNearbySnapshot {
        let inner = lock_desktop_nearby_inner(&self.inner);
        snapshot_locked(&inner)
    }

    /// Wipe every event currently in the mailbag (our outbound queue
    /// + items we're forwarding for others). Independent of the
    ///   on/off toggle so the user can clear without disabling future
    ///   sync; the toggle controls whether the bag accepts new items.
    pub fn empty_mailbag(&self) {
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            inner.own_outbound.clear();
            inner.forwarded.clear();
        }
        self.notify();
    }

    pub fn publish(&self, event_id: String, kind: u32, created_at_secs: u64, event_json: String) {
        let record = StoredNearbyEvent {
            id: event_id.clone(),
            kind,
            created_at_secs,
            author_pubkey_hex: event_author_hex(&event_json),
            event_json,
        };
        let mailbag_enabled = self.app.state().preferences.nearby_mailbag_enabled;
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if mailbag_enabled {
                inner.own_outbound.insert(event_id.clone(), record.clone());
                inner.forwarded.remove(&event_id);
                if kind == 0 {
                    if let Some(profile) = NearbyProfileEvent::from_event_json(&record.event_json) {
                        inner.own_profile_event_id = Some(event_id);
                        inner.known_profiles.insert(profile.id.clone(), profile);
                    }
                }
                prune_mailbags(&mut inner);
            }
            if !inner.visible {
                return;
            }
        }
        if kind == 0 {
            self.send_hello(None);
        }
        self.send_event(&record, None);
    }
}

impl DesktopNearbyService {
    fn clone_handles(&self) -> DesktopNearbyRuntime {
        DesktopNearbyRuntime {
            app: self.app.clone(),
            inner: self.inner.clone(),
            observer: self.observer.clone(),
        }
    }

    fn set_status(&self, status: &str) {
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            inner.status = status.to_string();
        }
        self.notify();
    }

    fn notify(&self) {
        let snapshot = self.snapshot();
        self.observer.desktop_nearby_changed(snapshot);
    }

    fn announce_to_connected_peers(&self) {
        self.send_hello(None);
        self.send_inventory(None);
    }

    fn send_hello(&self, excluding_peer_id: Option<&str>) {
        let (nonce, name, visible) = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            (
                inner.local_nonce.clone(),
                inner.local_name.clone(),
                inner.visible,
            )
        };
        if !visible {
            return;
        }
        self.send_envelope(
            &NearbyEnvelope::hello(Some(nonce), Some(name)),
            excluding_peer_id,
        );
    }

    fn send_inventory(&self, excluding_peer_id: Option<&str>) {
        if !self.app.state().preferences.nearby_mailbag_enabled {
            // Mailbag off → don't advertise our bag to peers (the
            // contents survive the toggle for when it flips back on,
            // but until then we're silent on the sync wire).
            return;
        }
        let records = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            mailbag_events(&inner)
        };
        if records.is_empty() {
            return;
        }
        for record in records.into_iter().take(200) {
            self.send_envelope(
                &NearbyEnvelope::inv(NearbyInventoryItem {
                    id: record.id,
                    author: record.author_pubkey_hex,
                    kind: u64::from(record.kind),
                    created_at: record.created_at_secs,
                    size: record.event_json.len() as u64,
                }),
                excluding_peer_id,
            );
        }
    }

    fn send_want(&self, ids: Vec<String>, excluding_peer_id: Option<&str>) {
        if ids.is_empty() {
            return;
        }
        for id in ids.into_iter().take(64) {
            self.send_envelope(&NearbyEnvelope::want(id), excluding_peer_id);
        }
    }

    fn send_event(&self, record: &StoredNearbyEvent, excluding_peer_id: Option<&str>) {
        self.send_envelope(
            &NearbyEnvelope::event(record.event_json.clone()),
            excluding_peer_id,
        );
    }

    fn send_presence(&self, remote_nonce: &str) {
        let (peer_id, local_nonce, profile_event_id) = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            (
                inner.peer_id.clone(),
                inner.local_nonce.clone(),
                inner.own_profile_event_id.clone().unwrap_or_default(),
            )
        };
        let event_json = self.app.build_nearby_presence_event_json(
            peer_id,
            local_nonce,
            remote_nonce.to_string(),
            profile_event_id,
        );
        if event_json.trim().is_empty() {
            return;
        }
        let record = StoredNearbyEvent {
            id: String::new(),
            kind: NEARBY_PRESENCE_KIND,
            created_at_secs: now_secs(),
            event_json,
            author_pubkey_hex: None,
        };
        self.send_event(&record, None);
    }

    fn send_presence_if_needed(&self, remote_nonce: &str, response_key: &str, force: bool) {
        let key = format!("{response_key}|{remote_nonce}");
        let now = Instant::now();
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if !force
                && inner
                    .presence_sent_at
                    .get(&key)
                    .is_some_and(|last| now.duration_since(*last) < PRESENCE_RESEND_INTERVAL)
            {
                return;
            }
            inner.presence_sent_at.insert(key, now);
            inner
                .presence_sent_at
                .retain(|_, last| now.duration_since(*last) < PRESENCE_RESEND_INTERVAL * 2);
        }
        self.send_presence(remote_nonce);
    }

    fn send_inventory_after_hello_if_needed(&self, response_key: &str, force: bool) {
        let now = Instant::now();
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if !force
                && inner
                    .peer_inventory_sent_at
                    .get(response_key)
                    .is_some_and(|last| now.duration_since(*last) < INVENTORY_RESEND_INTERVAL)
            {
                return;
            }
            inner
                .peer_inventory_sent_at
                .insert(response_key.to_string(), now);
            inner
                .peer_inventory_sent_at
                .retain(|_, last| now.duration_since(*last) < INVENTORY_RESEND_INTERVAL * 2);
        }
        self.send_inventory(None);
    }

    fn send_envelope(&self, envelope: &NearbyEnvelope, excluding_peer_id: Option<&str>) {
        let visible = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            inner.visible
        };
        if !visible {
            return;
        }
        let Some(frame) = encode_nearby_envelope_frame(envelope) else {
            return;
        };
        if frame.is_empty() || frame.len() > SINGLE_FRAME_BYTES {
            return;
        }
        self.send_frame(&frame, excluding_peer_id);
    }

    fn send_frame(&self, frame: &[u8], excluding_peer_id: Option<&str>) {
        let writers = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            inner
                .connections
                .iter()
                .filter_map(|(id, connection)| {
                    if excluding_peer_id.is_some()
                        && connection.peer_id.as_deref() == excluding_peer_id
                    {
                        return None;
                    }
                    Some((id.clone(), connection.writer.clone()))
                })
                .collect::<Vec<_>>()
        };
        let mut failed = Vec::new();
        for (id, writer) in writers {
            let result = writer
                .lock()
                .map_err(|_| ())
                .and_then(|mut stream| stream.write_all(frame).map_err(|_| ()));
            if result.is_err() {
                failed.push(id);
            }
        }
        if !failed.is_empty() {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            for id in failed {
                inner.connections.remove(&id);
            }
            inner.status = if inner.visible && inner.connections.is_empty() {
                "Visible".to_string()
            } else if inner.visible {
                "Connected".to_string()
            } else {
                "Off".to_string()
            };
        }
    }
}

#[derive(Clone)]
struct DesktopNearbyRuntime {
    app: Arc<FfiApp>,
    inner: Arc<Mutex<DesktopNearbyInner>>,
    observer: Arc<dyn DesktopNearbyObserver>,
}

impl DesktopNearbyRuntime {
    fn accept_loop(&self, listener: TcpListener, stop_flag: Arc<AtomicBool>) {
        while !stop_flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    if is_private_socket_addr(&addr) {
                        self.add_connection(stream, None, Some(addr.to_string()));
                    } else {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
    }

    fn mdns_loop(
        &self,
        socket: UdpSocket,
        local_addr: Ipv4Addr,
        port: u16,
        stop_flag: Arc<AtomicBool>,
    ) {
        let mut buffer = [0u8; 1500];
        let mut last_query = Instant::now() - MDNS_QUERY_INTERVAL;
        let mut last_announce = Instant::now() - MDNS_ANNOUNCE_INTERVAL;
        let mut last_hello = Instant::now() - HELLO_INTERVAL;
        while !stop_flag.load(Ordering::SeqCst) {
            let now = Instant::now();
            if now.duration_since(last_query) >= MDNS_QUERY_INTERVAL {
                let _ = socket.send_to(&mdns_query_packet(), mdns_addr());
                last_query = now;
            }
            if now.duration_since(last_announce) >= MDNS_ANNOUNCE_INTERVAL {
                let packet = {
                    let inner = lock_desktop_nearby_inner(&self.inner);
                    mdns_response_packet(&inner.peer_id, local_addr, port)
                };
                let _ = socket.send_to(&packet, mdns_addr());
                last_announce = now;
            }
            if now.duration_since(last_hello) >= HELLO_INTERVAL {
                self.send_hello(None);
                last_hello = now;
            }
            match socket.recv_from(&mut buffer) {
                Ok((count, source)) => {
                    if count == 0 {
                        continue;
                    }
                    let Some(packet_bytes) = buffer.get(..count) else {
                        continue;
                    };
                    if let Some(packet) = MdnsPacket::parse(packet_bytes) {
                        if packet.queries_service() {
                            let response = {
                                let inner = lock_desktop_nearby_inner(&self.inner);
                                mdns_response_packet(&inner.peer_id, local_addr, port)
                            };
                            let _ = socket.send_to(&response, mdns_addr());
                        }
                        self.handle_mdns_packet(packet, source);
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
            self.run_maintenance();
        }
    }

    fn handle_mdns_packet(&self, packet: MdnsPacket, _source: SocketAddr) {
        let own_peer_id = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            inner.peer_id.clone()
        };
        let mut targets = Vec::new();
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            for instance_name in packet.ptr_instances {
                if instance_name.contains(&own_peer_id) {
                    continue;
                }
                let peer_id = mdns_peer_id(&instance_name);
                inner
                    .mdns_instances
                    .entry(instance_name)
                    .or_default()
                    .peer_id = peer_id;
            }
            for (name, target, port) in packet.srv_records {
                let instance = inner.mdns_instances.entry(name).or_default();
                instance.target = Some(target);
                instance.port = Some(port);
            }
            for (host, addr) in packet.a_records {
                for instance in inner.mdns_instances.values_mut() {
                    if instance.target.as_deref() == Some(host.as_str()) {
                        instance.addr = Some(addr);
                    }
                }
            }
            let discovered = inner
                .mdns_instances
                .values()
                .filter_map(|instance| {
                    Some((instance.addr?, instance.port?, instance.peer_id.clone()))
                })
                .collect::<Vec<_>>();
            for (addr, port, remote_peer_id) in discovered {
                let key = format!("{addr}:{port}");
                if inner.endpoint_keys.insert(key.clone()) {
                    targets.push((addr, port, key, remote_peer_id));
                }
            }
        }
        for (addr, port, key, remote_peer_id) in targets {
            if !is_private_ipv4(addr) {
                continue;
            }
            match TcpStream::connect_timeout(
                &SocketAddr::V4(SocketAddrV4::new(addr, port)),
                Duration::from_secs(3),
            ) {
                Ok(stream) => self.add_connection(stream, remote_peer_id, Some(key)),
                Err(_) => {
                    let mut inner = lock_desktop_nearby_inner(&self.inner);
                    inner.endpoint_keys.remove(&key);
                }
            }
        }
    }

    fn add_connection(
        &self,
        stream: TcpStream,
        remote_peer_id: Option<String>,
        endpoint_key: Option<String>,
    ) {
        let _ = stream.set_nodelay(true);
        let writer_stream = match stream.try_clone() {
            Ok(writer) => writer,
            Err(_) => return,
        };
        let connection_id = random_id();
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if !inner.visible {
                return;
            }
            if let Some(key) = endpoint_key.as_ref() {
                inner.endpoint_keys.insert(key.clone());
            }
            inner.connections.insert(
                connection_id.clone(),
                DesktopNearbyConnection {
                    writer: Arc::new(Mutex::new(writer_stream)),
                    peer_id: remote_peer_id,
                },
            );
            inner.status = "Connected".to_string();
        }
        self.notify();
        self.send_hello(None);
        let runtime = self.clone();
        thread::spawn(move || runtime.read_loop(connection_id, stream));
    }

    fn read_loop(&self, connection_id: String, mut stream: TcpStream) {
        loop {
            let mut header = [0u8; NEARBY_HEADER_BYTES];
            if stream.read_exact(&mut header).is_err() {
                break;
            }
            let body_len = self.app.nearby_frame_body_len_from_header(header.to_vec());
            if body_len <= 0 || body_len as usize > MAX_FRAME_BODY_BYTES {
                break;
            }
            let mut body = vec![0u8; body_len as usize];
            if stream.read_exact(&mut body).is_err() {
                break;
            }
            let mut frame = header.to_vec();
            frame.extend(body);
            self.ingest_frame(&connection_id, frame);
        }
        self.close_connection(&connection_id);
    }

    fn close_connection(&self, connection_id: &str) {
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            inner.connections.remove(connection_id);
            inner.connection_nonces.remove(connection_id);
            inner.status = if !inner.visible {
                "Off".to_string()
            } else if inner.connections.is_empty() {
                "Visible".to_string()
            } else {
                "Connected".to_string()
            };
        }
        self.notify();
    }

    fn ingest_frame(&self, connection_id: &str, frame: Vec<u8>) {
        let Some(envelope) = decode_nearby_envelope_frame(&frame) else {
            return;
        };
        let (own_peer_id, remote_peer_id) = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            (
                inner.peer_id.clone(),
                inner
                    .connections
                    .get(connection_id)
                    .and_then(|connection| connection.peer_id.clone()),
            )
        };
        if remote_peer_id.as_deref() == Some(own_peer_id.as_str()) {
            return;
        }
        if let Some(remote_peer_id) = remote_peer_id.as_deref() {
            self.touch_peer(remote_peer_id);
        }

        match envelope {
            NearbyEnvelope::Hello { nonce, name, .. } => self.handle_hello(
                connection_id,
                remote_peer_id.as_deref(),
                nonce,
                name.as_deref(),
            ),
            NearbyEnvelope::Inv { id, size, .. } => self.handle_inventory(&id, size),
            NearbyEnvelope::Want { id, .. } => self.handle_want(&id),
            NearbyEnvelope::Event { event_json, .. } => {
                self.handle_event_envelope(&event_json, remote_peer_id.as_deref(), connection_id)
            }
        }
    }

    fn handle_hello(
        &self,
        connection_id: &str,
        remote_peer_id: Option<&str>,
        remote_nonce: Option<String>,
        name: Option<&str>,
    ) {
        let (was_new, nonce_changed) = {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if let Some(nonce) = remote_nonce.as_ref() {
                inner
                    .connection_nonces
                    .insert(connection_id.to_string(), nonce.clone());
            }
            if let Some(remote_peer_id) = remote_peer_id {
                if let Some(connection) = inner.connections.get_mut(connection_id) {
                    connection.peer_id = Some(remote_peer_id.to_string());
                }
                let previous_nonce = inner.peer_nonces.get(remote_peer_id).cloned();
                if let Some(nonce) = remote_nonce.as_ref() {
                    inner
                        .peer_nonces
                        .insert(remote_peer_id.to_string(), nonce.clone());
                }
                let was_new = remember_peer(&mut inner, remote_peer_id, name, None);
                let nonce_changed = remote_nonce.is_some() && remote_nonce != previous_nonce;
                if was_new || nonce_changed {
                    inner.status = nearby_status(&inner);
                }
                (was_new, nonce_changed)
            } else {
                inner.status = nearby_status(&inner);
                (false, false)
            }
        };
        if was_new {
            self.notify();
            self.send_hello(None);
        }
        if let Some(nonce) = remote_nonce.as_deref() {
            let response_key = remote_peer_id.unwrap_or(connection_id);
            self.send_presence_if_needed(nonce, response_key, was_new || nonce_changed);
        }
        self.send_inventory_after_hello_if_needed(
            remote_peer_id.unwrap_or(connection_id),
            was_new || nonce_changed,
        );
    }

    fn handle_inventory(&self, id: &str, size: u64) {
        if !self.app.state().preferences.nearby_mailbag_enabled {
            return;
        }
        let wanted = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            id.len() == 64
                && (1..=MAX_FRAME_BODY_BYTES as u64).contains(&size)
                && !inner.own_outbound.contains_key(id)
                && !inner.forwarded.contains_key(id)
        };
        if wanted {
            self.send_want(vec![id.to_string()], None);
        }
    }

    fn handle_want(&self, id: &str) {
        if !self.app.state().preferences.nearby_mailbag_enabled {
            return;
        }
        let record = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            inner
                .own_outbound
                .get(id)
                .or_else(|| inner.forwarded.get(id))
                .cloned()
        };
        if let Some(record) = record {
            self.send_event(&record, None);
        }
    }

    fn handle_event_envelope(
        &self,
        event_json: &str,
        remote_peer_id: Option<&str>,
        connection_id: &str,
    ) {
        if event_json.len() > MAX_FRAME_BODY_BYTES {
            return;
        }
        let Some(record) = StoredNearbyEvent::from_event_json(event_json) else {
            return;
        };
        if record.kind == NEARBY_PRESENCE_KIND {
            if self.handle_presence_event(event_json, remote_peer_id, connection_id) {
                self.notify();
            }
            return;
        }
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if let Some(existing) = inner
                .own_outbound
                .get(&record.id)
                .or_else(|| inner.forwarded.get(&record.id))
                .cloned()
            {
                remember_profile(&mut inner, &existing.event_json, remote_peer_id);
                return;
            }
        }
        if !self.app.ingest_nearby_event_json(event_json.to_string()) {
            return;
        }
        let mailbag_enabled = self.app.state().preferences.nearby_mailbag_enabled;
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            remember_profile(&mut inner, event_json, remote_peer_id);
            // Mailbag off → ingest the event for the local app but
            // don't store it for forwarding to other peers. Existing
            // entries are left alone so the bag survives the toggle.
            if mailbag_enabled {
                inner.forwarded.insert(record.id.clone(), record);
                prune_mailbags(&mut inner);
            }
        }
        self.notify();
        self.send_inventory(remote_peer_id);
    }

    fn handle_presence_event(
        &self,
        event_json: &str,
        remote_peer_id: Option<&str>,
        connection_id: &str,
    ) -> bool {
        let peer_id = remote_peer_id
            .map(str::to_string)
            .or_else(|| nearby_presence_peer_id(event_json));
        let Some(peer_id) = peer_id else {
            return false;
        };
        let (local_nonce, nonce_candidates) = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            let nonce_candidates = if let Some(remote_nonce) =
                remote_peer_id.and_then(|peer_id| inner.peer_nonces.get(peer_id))
            {
                vec![(None, remote_nonce.clone())]
            } else {
                let mut candidates = Vec::new();
                let mut seen = HashSet::new();
                if let Some(remote_nonce) = inner.connection_nonces.get(connection_id) {
                    candidates.push((Some(connection_id.to_string()), remote_nonce.clone()));
                    seen.insert(connection_id.to_string());
                }
                for (key, nonce) in &inner.connection_nonces {
                    if seen.insert(key.clone()) {
                        candidates.push((Some(key.clone()), nonce.clone()));
                    }
                }
                candidates
            };
            if nonce_candidates.is_empty() {
                return false;
            }
            (inner.local_nonce.clone(), nonce_candidates)
        };
        let mut verified = None;
        for (nonce_key, remote_nonce) in nonce_candidates {
            let result = self.app.verify_nearby_presence_event_json(
                event_json.to_string(),
                peer_id.clone(),
                local_nonce.clone(),
                remote_nonce,
            );
            let Ok(value) = serde_json::from_str::<Value>(&result) else {
                continue;
            };
            let owner_pubkey_hex = value
                .get("owner_pubkey_hex")
                .and_then(Value::as_str)
                .filter(|v| v.len() == 64)
                .map(str::to_string);
            let Some(owner_pubkey_hex) = owner_pubkey_hex else {
                continue;
            };
            let profile_event_id = value
                .get("profile_event_id")
                .and_then(Value::as_str)
                .filter(|v| v.len() == 64)
                .map(str::to_string);
            verified = Some((nonce_key, owner_pubkey_hex, profile_event_id));
            break;
        }
        let Some((nonce_key, owner_pubkey_hex, profile_event_id)) = verified else {
            return false;
        };
        {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if let Some(connection) = inner.connections.get_mut(connection_id) {
                connection.peer_id = Some(peer_id.clone());
            }
            inner.connection_nonces.remove(connection_id);
            if let Some(nonce_key) = nonce_key {
                inner.connection_nonces.remove(&nonce_key);
            }
            remember_presence(&mut inner, &peer_id, owner_pubkey_hex, profile_event_id);
        }
        true
    }

    fn touch_peer(&self, peer_id: &str) {
        let mut inner = lock_desktop_nearby_inner(&self.inner);
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.last_seen = Instant::now();
        }
    }

    fn run_maintenance(&self) {
        let stale = {
            let mut inner = lock_desktop_nearby_inner(&self.inner);
            if !inner.visible {
                return;
            }
            let now = Instant::now();
            let stale = inner
                .peers
                .values()
                .filter(|peer| now.duration_since(peer.last_seen) > PEER_TTL)
                .map(|peer| peer.id.clone())
                .collect::<Vec<_>>();
            for peer_id in &stale {
                inner.peers.remove(peer_id);
                inner.peer_nonces.remove(peer_id);
                inner.peer_inventory_sent_at.remove(peer_id);
                inner
                    .presence_sent_at
                    .retain(|key, _| !key.starts_with(&format!("{peer_id}|")));
            }
            if !stale.is_empty() {
                inner.status = nearby_status(&inner);
            }
            stale
        };
        if !stale.is_empty() {
            self.notify();
        }
    }

    fn notify(&self) {
        let snapshot = {
            let inner = lock_desktop_nearby_inner(&self.inner);
            snapshot_locked(&inner)
        };
        self.observer.desktop_nearby_changed(snapshot);
    }

    fn send_hello(&self, excluding_peer_id: Option<&str>) {
        let service = DesktopNearbyService {
            app: self.app.clone(),
            observer: self.observer.clone(),
            inner: self.inner.clone(),
        };
        service.send_hello(excluding_peer_id);
    }

    fn send_inventory(&self, excluding_peer_id: Option<&str>) {
        let service = DesktopNearbyService {
            app: self.app.clone(),
            observer: self.observer.clone(),
            inner: self.inner.clone(),
        };
        service.send_inventory(excluding_peer_id);
    }

    fn send_want(&self, ids: Vec<String>, excluding_peer_id: Option<&str>) {
        let service = DesktopNearbyService {
            app: self.app.clone(),
            observer: self.observer.clone(),
            inner: self.inner.clone(),
        };
        service.send_want(ids, excluding_peer_id);
    }

    fn send_event(&self, record: &StoredNearbyEvent, excluding_peer_id: Option<&str>) {
        let service = DesktopNearbyService {
            app: self.app.clone(),
            observer: self.observer.clone(),
            inner: self.inner.clone(),
        };
        service.send_event(record, excluding_peer_id);
    }

    fn send_presence_if_needed(&self, remote_nonce: &str, response_key: &str, force: bool) {
        let service = DesktopNearbyService {
            app: self.app.clone(),
            observer: self.observer.clone(),
            inner: self.inner.clone(),
        };
        service.send_presence_if_needed(remote_nonce, response_key, force);
    }

    fn send_inventory_after_hello_if_needed(&self, response_key: &str, force: bool) {
        let service = DesktopNearbyService {
            app: self.app.clone(),
            observer: self.observer.clone(),
            inner: self.inner.clone(),
        };
        service.send_inventory_after_hello_if_needed(response_key, force);
    }
}

fn snapshot_locked(inner: &DesktopNearbyInner) -> DesktopNearbySnapshot {
    let now = Instant::now();
    let mut peers = inner
        .peers
        .values()
        .map(|peer| DesktopNearbyPeerSnapshot {
            id: peer.id.clone(),
            name: peer.name.clone(),
            owner_pubkey_hex: peer.owner_pubkey_hex.clone(),
            picture_url: peer.picture_url.clone(),
            profile_event_id: peer.profile_event_id.clone(),
            last_seen_secs: now.duration_since(peer.last_seen).as_secs(),
        })
        .collect::<Vec<_>>();
    peers.sort_by(|a, b| {
        deterministic_peer_sort_key(a)
            .cmp(&deterministic_peer_sort_key(b))
            .then_with(|| a.id.cmp(&b.id))
    });
    DesktopNearbySnapshot {
        visible: inner.visible,
        status: inner.status.clone(),
        peers,
    }
}

fn deterministic_peer_sort_key(peer: &DesktopNearbyPeerSnapshot) -> String {
    peer.owner_pubkey_hex
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .map(str::to_lowercase)
        .unwrap_or_else(|| format!("peer:{}", peer.id.to_lowercase()))
}

fn remember_peer(
    inner: &mut DesktopNearbyInner,
    peer_id: &str,
    name: Option<&str>,
    profile_event_id: Option<String>,
) -> bool {
    let existing = inner.peers.get(peer_id).cloned();
    let profile_event_id = profile_event_id.or_else(|| existing.as_ref()?.profile_event_id.clone());
    inner.peers.insert(
        peer_id.to_string(),
        DesktopNearbyPeer {
            id: peer_id.to_string(),
            name: nearby_peer_name(
                name,
                existing
                    .as_ref()
                    .and_then(|peer| peer.owner_pubkey_hex.as_deref()),
                None,
                existing.as_ref().map(|peer| peer.name.as_str()),
            ),
            owner_pubkey_hex: existing
                .as_ref()
                .and_then(|peer| peer.owner_pubkey_hex.clone()),
            picture_url: existing.as_ref().and_then(|peer| peer.picture_url.clone()),
            profile_event_id: profile_event_id.clone(),
            last_seen: Instant::now(),
        },
    );
    if let Some(profile_event_id) = profile_event_id {
        if let Some(profile) = inner.known_profiles.get(&profile_event_id).cloned() {
            apply_profile(inner, peer_id, &profile);
        }
    }
    existing.is_none()
}

fn remember_profile(
    inner: &mut DesktopNearbyInner,
    event_json: &str,
    remote_peer_id: Option<&str>,
) {
    let Some(profile) = NearbyProfileEvent::from_event_json(event_json) else {
        return;
    };
    inner
        .known_profiles
        .insert(profile.id.clone(), profile.clone());
    if let Some(peer_id) = remote_peer_id {
        if !inner.peers.contains_key(peer_id) {
            inner.peers.insert(
                peer_id.to_string(),
                DesktopNearbyPeer {
                    id: peer_id.to_string(),
                    name: nearby_peer_name(
                        None,
                        Some(profile.owner_pubkey_hex.as_str()),
                        profile.display_name.as_deref(),
                        None,
                    ),
                    owner_pubkey_hex: Some(profile.owner_pubkey_hex.clone()),
                    picture_url: profile.picture_url.clone(),
                    profile_event_id: Some(profile.id.clone()),
                    last_seen: Instant::now(),
                },
            );
        }
        apply_profile(inner, peer_id, &profile);
        inner.status = nearby_status(inner);
    }
}

fn remember_presence(
    inner: &mut DesktopNearbyInner,
    peer_id: &str,
    owner_pubkey_hex: String,
    profile_event_id: Option<String>,
) {
    let Some(peer) = inner.peers.get_mut(peer_id) else {
        return;
    };
    peer.owner_pubkey_hex = Some(owner_pubkey_hex);
    if profile_event_id.is_some() {
        peer.profile_event_id = profile_event_id;
    }
    if let Some(owner) = peer.owner_pubkey_hex.clone() {
        let existing_name = peer.name.clone();
        peer.name = nearby_peer_name(
            None,
            Some(owner.as_str()),
            None,
            Some(existing_name.as_str()),
        );
    }
    peer.last_seen = Instant::now();
    if let Some(profile_id) = peer.profile_event_id.clone() {
        if let Some(profile) = inner.known_profiles.get(&profile_id).cloned() {
            apply_profile(inner, peer_id, &profile);
        }
    }
    inner.status = nearby_status(inner);
}

fn apply_profile(inner: &mut DesktopNearbyInner, peer_id: &str, profile: &NearbyProfileEvent) {
    let Some(peer) = inner.peers.get_mut(peer_id) else {
        return;
    };
    if let Some(owner) = peer.owner_pubkey_hex.as_ref() {
        if !owner.eq_ignore_ascii_case(&profile.owner_pubkey_hex) {
            return;
        }
    }
    if let Some(profile_id) = peer.profile_event_id.as_ref() {
        if profile_id != &profile.id {
            return;
        }
    }
    let existing_name = peer.name.clone();
    peer.name = nearby_peer_name(
        None,
        Some(profile.owner_pubkey_hex.as_str()),
        profile.display_name.as_deref(),
        Some(existing_name.as_str()),
    );
    peer.owner_pubkey_hex = Some(profile.owner_pubkey_hex.clone());
    peer.picture_url = profile
        .picture_url
        .clone()
        .or_else(|| peer.picture_url.clone());
    peer.profile_event_id = Some(profile.id.clone());
    peer.last_seen = Instant::now();
}

fn nearby_status(inner: &DesktopNearbyInner) -> String {
    match inner.peers.len() {
        0 => "Visible".to_string(),
        1 => "1 nearby".to_string(),
        count => format!("{count} nearby"),
    }
}

fn mailbag_events(inner: &DesktopNearbyInner) -> Vec<StoredNearbyEvent> {
    let mut records = inner
        .own_outbound
        .values()
        .chain(inner.forwarded.values())
        .cloned()
        .collect::<Vec<_>>();
    records.sort_by_key(|record| std::cmp::Reverse(record.created_at_secs));
    if let Some(profile_id) = inner.own_profile_event_id.as_ref() {
        if let Some(index) = records.iter().position(|record| &record.id == profile_id) {
            let profile = records.remove(index);
            records.insert(0, profile);
        }
    }
    records
}

fn prune_mailbags(inner: &mut DesktopNearbyInner) {
    prune_bag(
        &mut inner.own_outbound,
        inner.own_profile_event_id.as_deref(),
    );
    prune_bag(&mut inner.forwarded, None);
}

fn prune_bag(bag: &mut HashMap<String, StoredNearbyEvent>, preserving_id: Option<&str>) {
    if bag.len() <= MAX_MAILBAG_EVENTS {
        return;
    }
    let mut records = bag.values().cloned().collect::<Vec<_>>();
    records.sort_by_key(|record| std::cmp::Reverse(record.created_at_secs));
    let keep = records
        .into_iter()
        .take(MAX_MAILBAG_EVENTS)
        .map(|record| record.id)
        .collect::<HashSet<_>>();
    bag.retain(|id, _| keep.contains(id) || preserving_id == Some(id.as_str()));
}

impl StoredNearbyEvent {
    fn from_event_json(event_json: &str) -> Option<Self> {
        let value = serde_json::from_str::<Value>(event_json).ok()?;
        let id = value.get("id")?.as_str()?.to_string();
        let kind = value.get("kind")?.as_u64()? as u32;
        let created_at_secs = value.get("created_at")?.as_u64()?;
        if id.len() != 64 {
            return None;
        }
        Some(Self {
            id,
            kind,
            created_at_secs,
            author_pubkey_hex: event_author_hex(event_json),
            event_json: event_json.to_string(),
        })
    }
}

impl NearbyProfileEvent {
    fn from_event_json(event_json: &str) -> Option<Self> {
        let event = serde_json::from_str::<Value>(event_json).ok()?;
        if event.get("kind")?.as_u64()? != 0 {
            return None;
        }
        let id = event.get("id")?.as_str()?.to_string();
        let owner_pubkey_hex = event.get("pubkey")?.as_str()?.to_string();
        if id.len() != 64 || owner_pubkey_hex.len() != 64 {
            return None;
        }
        let content = event.get("content")?.as_str()?;
        let metadata = serde_json::from_str::<Value>(content).ok()?;
        let display_name = metadata
            .get("display_name")
            .or_else(|| metadata.get("name"))
            .and_then(Value::as_str)
            .and_then(|name| clean_optional_name(Some(name)));
        let picture_url = metadata
            .get("picture")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
            .map(str::to_string);
        Some(Self {
            id,
            owner_pubkey_hex,
            display_name,
            picture_url,
        })
    }
}

fn private_local_ipv4() -> Option<Ipv4Addr> {
    for target in ["8.8.8.8:80", "1.1.1.1:80"] {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        if socket.connect(target).is_ok() {
            if let Ok(SocketAddr::V4(addr)) = socket.local_addr() {
                let ip = *addr.ip();
                if is_private_ipv4(ip) {
                    return Some(ip);
                }
            }
        }
    }
    None
}

fn mdns_socket(local_addr: Ipv4Addr) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    {
        let _ = socket.set_reuse_port(true);
    }
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT).into())?;
    let udp: UdpSocket = socket.into();
    udp.join_multicast_v4(&MDNS_GROUP, &local_addr)?;
    udp.set_multicast_loop_v4(true)?;
    udp.set_multicast_ttl_v4(255)?;
    udp.set_read_timeout(Some(Duration::from_millis(500)))?;
    Ok(udp)
}

fn mdns_addr() -> SocketAddrV4 {
    SocketAddrV4::new(MDNS_GROUP, MDNS_PORT)
}

fn mdns_query_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    write_dns_name(&mut packet, SERVICE_TYPE);
    packet.extend_from_slice(&12u16.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet
}

fn mdns_response_packet(peer_id: &str, local_addr: Ipv4Addr, port: u16) -> Vec<u8> {
    let instance = mdns_instance_name(peer_id);
    let host = mdns_host_name(peer_id);
    let mut packet = Vec::new();
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0x8400u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&4u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());

    write_dns_name(&mut packet, SERVICE_TYPE);
    packet.extend_from_slice(&12u16.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet.extend_from_slice(&120u32.to_be_bytes());
    let mut ptr = Vec::new();
    write_dns_name(&mut ptr, &instance);
    packet.extend_from_slice(&(ptr.len() as u16).to_be_bytes());
    packet.extend_from_slice(&ptr);

    write_dns_name(&mut packet, &instance);
    packet.extend_from_slice(&33u16.to_be_bytes());
    packet.extend_from_slice(&0x8001u16.to_be_bytes());
    packet.extend_from_slice(&120u32.to_be_bytes());
    let mut srv = Vec::new();
    srv.extend_from_slice(&0u16.to_be_bytes());
    srv.extend_from_slice(&0u16.to_be_bytes());
    srv.extend_from_slice(&port.to_be_bytes());
    write_dns_name(&mut srv, &host);
    packet.extend_from_slice(&(srv.len() as u16).to_be_bytes());
    packet.extend_from_slice(&srv);

    write_dns_name(&mut packet, &instance);
    packet.extend_from_slice(&16u16.to_be_bytes());
    packet.extend_from_slice(&0x8001u16.to_be_bytes());
    packet.extend_from_slice(&120u32.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet.push(0);

    write_dns_name(&mut packet, &host);
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet.extend_from_slice(&0x8001u16.to_be_bytes());
    packet.extend_from_slice(&120u32.to_be_bytes());
    packet.extend_from_slice(&4u16.to_be_bytes());
    packet.extend_from_slice(&local_addr.octets());
    packet
}

fn write_dns_name(packet: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.') {
        packet.push(label.len().min(63) as u8);
        packet.extend_from_slice(
            label
                .as_bytes()
                .get(..label.len().min(63))
                .unwrap_or_default(),
        );
    }
    packet.push(0);
}

fn mdns_instance_name(peer_id: &str) -> String {
    format!("iris-{peer_id}.{SERVICE_TYPE}")
}

fn mdns_peer_id(instance_name: &str) -> Option<String> {
    let normalized = normalize_dns_name(instance_name);
    let suffix = format!(".{}", normalize_dns_name(SERVICE_TYPE));
    let peer_id = normalized.strip_prefix("iris-")?.strip_suffix(&suffix)?;
    (!peer_id.is_empty()).then(|| peer_id.to_string())
}

fn mdns_host_name(peer_id: &str) -> String {
    format!("iris-{peer_id}.local")
}

#[derive(Default)]
struct MdnsPacket {
    questions: Vec<String>,
    ptr_instances: Vec<String>,
    srv_records: Vec<(String, String, u16)>,
    a_records: Vec<(String, Ipv4Addr)>,
}

impl MdnsPacket {
    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 {
            return None;
        }
        let qd = read_u16(bytes, 4)? as usize;
        let an = read_u16(bytes, 6)? as usize;
        let ns = read_u16(bytes, 8)? as usize;
        let ar = read_u16(bytes, 10)? as usize;
        let mut offset = 12;
        let mut packet = MdnsPacket::default();
        for _ in 0..qd {
            let (name, next) = read_dns_name(bytes, offset)?;
            offset = next.checked_add(4)?;
            packet.questions.push(name);
        }
        for _ in 0..(an + ns + ar) {
            let (name, next) = read_dns_name(bytes, offset)?;
            offset = next;
            let typ = read_u16(bytes, offset)?;
            let _class = read_u16(bytes, offset + 2)?;
            let _ttl = read_u32(bytes, offset + 4)?;
            let rdlen = read_u16(bytes, offset + 8)? as usize;
            offset += 10;
            let end = offset.checked_add(rdlen)?;
            if end > bytes.len() {
                return None;
            }
            match typ {
                12 if normalize_dns_name(&name) == SERVICE_TYPE => {
                    if let Some((target, _)) = read_dns_name(bytes, offset) {
                        packet.ptr_instances.push(target);
                    }
                }
                33 if rdlen >= 7 => {
                    let port = read_u16(bytes, offset + 4)?;
                    if let Some((target, _)) = read_dns_name(bytes, offset + 6) {
                        packet.srv_records.push((name, target, port));
                    }
                }
                1 if rdlen == 4 => {
                    let Some(addr_bytes) = bytes.get(offset..offset + 4) else {
                        continue;
                    };
                    let Ok(addr_bytes) = <[u8; 4]>::try_from(addr_bytes) else {
                        continue;
                    };
                    packet.a_records.push((name, Ipv4Addr::from(addr_bytes)));
                }
                _ => {}
            }
            offset = end;
        }
        Some(packet)
    }

    fn queries_service(&self) -> bool {
        self.questions
            .iter()
            .any(|name| normalize_dns_name(name) == SERVICE_TYPE)
    }
}

fn read_dns_name(bytes: &[u8], offset: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut cursor = offset;
    let mut next = None;
    for _ in 0..32 {
        let len = *bytes.get(cursor)?;
        if len & 0xc0 == 0xc0 {
            let second = *bytes.get(cursor + 1)? as usize;
            let pointer = (((len as usize) & 0x3f) << 8) | second;
            next.get_or_insert(cursor + 2);
            cursor = pointer;
            continue;
        }
        cursor += 1;
        if len == 0 {
            return Some((labels.join("."), next.unwrap_or(cursor)));
        }
        let end = cursor.checked_add(len as usize)?;
        let label = std::str::from_utf8(bytes.get(cursor..end)?).ok()?;
        labels.push(label.to_string());
        cursor = end;
    }
    None
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
    ]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
        *bytes.get(offset + 2)?,
        *bytes.get(offset + 3)?,
    ]))
}

fn normalize_dns_name(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn event_author_hex(event_json: &str) -> Option<String> {
    serde_json::from_str::<Value>(event_json)
        .ok()
        .and_then(|event| {
            event
                .get("pubkey")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| value.len() == 64)
                .map(str::to_string)
        })
}

fn nearby_presence_peer_id(event_json: &str) -> Option<String> {
    let event = serde_json::from_str::<Value>(event_json).ok()?;
    if event.get("kind")?.as_u64()? as u32 != NEARBY_PRESENCE_KIND {
        return None;
    }
    let content = event.get("content")?.as_str()?;
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|content| {
            content
                .get("peer_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn clean_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Iris".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn clean_optional_name(name: Option<&str>) -> Option<String> {
    let value = clean_name(name?);
    (value != "Iris").then_some(value)
}

fn nearby_peer_name(
    advertised_name: Option<&str>,
    owner_pubkey_hex: Option<&str>,
    profile_display_name: Option<&str>,
    existing_name: Option<&str>,
) -> String {
    if let Some(name) = clean_optional_name(profile_display_name) {
        return name;
    }
    if let Some(owner) = owner_pubkey_hex.and_then(nonempty) {
        return fallback_profile_name_for_identity(owner);
    }
    clean_optional_name(advertised_name)
        .or_else(|| clean_optional_name(existing_name))
        .unwrap_or_else(|| "Iris".to_string())
}

fn fallback_profile_name_for_identity(identity: &str) -> String {
    const ADJECTIVES: [&str; 12] = [
        "Amber", "Bright", "Calm", "Clear", "Golden", "Lunar", "Nova", "Quiet", "Silver", "Solar",
        "Velvet", "Wild",
    ];
    const NOUNS: [&str; 12] = [
        "Aurora", "Comet", "Echo", "Falcon", "Harbor", "Listener", "Otter", "Raven", "Signal",
        "Sparrow", "Tide", "Voyager",
    ];

    let trimmed = identity.trim();
    if trimmed.is_empty() {
        return "Quiet Listener".to_string();
    }

    let hash = trimmed.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let adjective = ADJECTIVES
        .get((hash as usize) % ADJECTIVES.len())
        .copied()
        .unwrap_or("Quiet");
    let noun = NOUNS
        .get(((hash as usize) / ADJECTIVES.len()) % NOUNS.len())
        .copied()
        .unwrap_or("Listener");
    format!("{adjective} {noun}")
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn is_private_socket_addr(addr: &SocketAddr) -> bool {
    match addr {
        SocketAddr::V4(addr) => is_private_ipv4(*addr.ip()),
        SocketAddr::V6(addr) => {
            let segments = addr.ip().segments();
            (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    match octets {
        [10, _, _, _] => true,
        [127, _, _, _] => true,
        [169, 254, _, _] => true,
        [172, second, _, _] if (16..=31).contains(&second) => true,
        [192, 168, _, _] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopDesktopNearbyObserver;

    impl DesktopNearbyObserver for NoopDesktopNearbyObserver {
        fn desktop_nearby_changed(&self, _snapshot: DesktopNearbySnapshot) {}
    }

    #[test]
    #[ignore = "LAN multicast discovery is host-network dependent and flakes in sandboxed CI"]
    fn desktop_lan_services_discover_each_other_on_same_host() {
        if private_local_ipv4().is_none() {
            eprintln!("skipping LAN nearby smoke: no private local IPv4 route");
            return;
        }

        let alice_dir = tempfile::TempDir::new().expect("alice temp dir");
        let bob_dir = tempfile::TempDir::new().expect("bob temp dir");
        let alice_app = FfiApp::new(
            alice_dir.path().to_string_lossy().to_string(),
            String::new(),
            "test".to_string(),
        );
        let bob_app = FfiApp::new(
            bob_dir.path().to_string_lossy().to_string(),
            String::new(),
            "test".to_string(),
        );
        let alice =
            DesktopNearbyService::new(alice_app.clone(), Arc::new(NoopDesktopNearbyObserver));
        let bob = DesktopNearbyService::new(bob_app.clone(), Arc::new(NoopDesktopNearbyObserver));

        alice.start("Alice".to_string());
        bob.start("Bob".to_string());

        let started = Instant::now();
        let mut alice_snapshot = alice.snapshot();
        let mut bob_snapshot = bob.snapshot();
        while started.elapsed() < Duration::from_secs(20) {
            alice_snapshot = alice.snapshot();
            bob_snapshot = bob.snapshot();
            if alice_snapshot.status == "Local network unavailable"
                || bob_snapshot.status == "Local network unavailable"
            {
                break;
            }
            if !alice_snapshot.peers.is_empty() && !bob_snapshot.peers.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }

        alice.stop();
        bob.stop();
        alice_app.shutdown();
        bob_app.shutdown();

        if alice_snapshot.status == "Local network unavailable"
            || bob_snapshot.status == "Local network unavailable"
        {
            eprintln!(
                "skipping LAN nearby smoke: local network unavailable (alice={}, bob={})",
                alice_snapshot.status, bob_snapshot.status
            );
            return;
        }

        assert!(
            !alice_snapshot.peers.is_empty() && !bob_snapshot.peers.is_empty(),
            "LAN nearby peers should discover each other; alice={alice_snapshot:?} bob={bob_snapshot:?}"
        );
    }

    #[test]
    fn verified_nearby_identity_beats_advertised_device_name() {
        let owner = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let expected = fallback_profile_name_for_identity(owner);

        assert_eq!(
            nearby_peer_name(Some("iPhone"), Some(owner), None, Some("iPhone")),
            expected
        );
    }

    #[test]
    fn advertised_profile_name_beats_identity_fallback() {
        let owner = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        assert_eq!(
            nearby_peer_name(Some("iPhone"), Some(owner), Some("Alice"), Some("iPhone")),
            "Alice"
        );
    }
}
