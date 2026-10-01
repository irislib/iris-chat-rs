//! Native binding smoke fixture. Uses fresh identities, loopback sockets and a
//! unique child directory; it never loads a user's account or attachment store.
use crate::core::direct_file_tcp::{
    start_direct_file_tcp, DirectFileEvent, DirectFileSender, TransferFile,
};
use fips_core::{FipsEndpoint, PeerIdentity};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinHandle;

const TOKEN: &str = "0102030405060708010203040506070801020304050607080102030405060708";

/// Runs actual FIPS-TCP file transport through the platform's native library.
/// Returns JSON so platform test runners can retain the same compact evidence.
#[uniffi::export]
pub fn run_direct_file_transfer_smoke(data_dir: String) -> String {
    let result = (|| -> Result<Value, String> {
        let parent = Path::new(&data_dir);
        if !parent.is_dir() {
            return Err("Smoke-test parent directory must exist".into());
        }
        let work = Scratch::new(parent)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(run(&work.0))
    })();
    match result {
        Ok(evidence) => evidence.to_string(),
        Err(error) => json!({"ok": false, "error": error}).to_string(),
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(parent: &Path) -> Result<Self, String> {
        let path = parent.join(format!(
            "iris-direct-files-smoke-{:016x}",
            rand::random::<u64>()
        ));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Peer {
    endpoint: Arc<FipsEndpoint>,
    identity: PeerIdentity,
    sender: DirectFileSender,
    events: flume::Receiver<DirectFileEvent>,
    task: JoinHandle<()>,
}
impl Peer {
    async fn new(address: SocketAddrV4) -> Result<Self, String> {
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
                .map_err(|error| error.to_string())?,
        );
        let identity =
            PeerIdentity::from_npub(endpoint.npub()).map_err(|error| error.to_string())?;
        let (tx, events) = flume::unbounded();
        let (sender, task) = start_direct_file_tcp(endpoint.clone(), tx).await?;
        Ok(Self {
            endpoint,
            identity,
            sender,
            events,
            task,
        })
    }
    async fn connect(&self, other: &Self) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .endpoint
                    .peers()
                    .await
                    .map_err(|error| error.to_string())?
                    .iter()
                    .any(|peer| peer.connected && peer.npub == other.endpoint.npub())
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| "FIPS authentication timed out".to_owned())?
    }
    async fn completed(&self) -> Result<Vec<String>, String> {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match self
                    .events
                    .recv_async()
                    .await
                    .map_err(|error| error.to_string())?
                {
                    DirectFileEvent::Completed { local_paths, .. } => return Ok(local_paths),
                    DirectFileEvent::Failed { error, .. } => return Err(error),
                    DirectFileEvent::Cancelled { .. } | DirectFileEvent::Declined { .. } => {
                        return Err("Transfer ended before completion".into())
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| "File transfer completion timed out".to_owned())?
    }
    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
        let _ = self.endpoint.shutdown().await;
    }
}

async fn run(directory: &Path) -> Result<Value, String> {
    let address = {
        let socket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|error| error.to_string())?;
        match socket.local_addr().map_err(|error| error.to_string())? {
            SocketAddr::V4(address) => address,
            _ => return Err("Expected IPv4 loopback".into()),
        }
    };
    let sender = Peer::new(address).await?;
    let receiver = match Peer::new(address).await {
        Ok(receiver) => receiver,
        Err(error) => {
            sender.stop().await;
            return Err(error);
        }
    };
    let result = transfer(directory, &sender, &receiver).await;
    sender.stop().await;
    receiver.stop().await;
    result
}

async fn transfer(directory: &Path, sender: &Peer, receiver: &Peer) -> Result<Value, String> {
    sender.connect(receiver).await?;
    receiver.connect(sender).await?;
    let sources = directory.join("source");
    let destination = directory.join("received");
    fs::create_dir(&sources).map_err(|error| error.to_string())?;
    fs::create_dir(&destination).map_err(|error| error.to_string())?;
    let content = [
        Vec::new(),
        (0..180_123).map(|i| ((i * 31) % 251) as u8).collect(),
        b"Native direct file transfer\n".repeat(337),
    ];
    let names = ["empty.txt", "binary.bin", "notes.txt"];
    let mut files = Vec::new();
    for (bytes, filename) in content.iter().zip(names) {
        let path = sources.join(filename);
        fs::write(&path, bytes).map_err(|error| error.to_string())?;
        files.push(TransferFile {
            filename: filename.into(),
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            path,
        });
    }
    sender.sender.register_offer(
        "native-smoke".into(),
        TOKEN.into(),
        vec![receiver.identity],
        files.clone(),
    )?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    if !sender.events.is_empty()
        || !receiver.events.is_empty()
        || fs::read_dir(&destination)
            .map_err(|error| error.to_string())?
            .next()
            .is_some()
    {
        return Err("File data or transfer activity appeared before acceptance".into());
    }
    receiver.sender.receive(
        "native-smoke".into(),
        TOKEN.into(),
        sender.identity,
        files.clone(),
        destination.clone(),
    )?;
    let (received, sent) = tokio::join!(receiver.completed(), sender.completed());
    let received = received?;
    sent?;
    if received.len() != files.len() {
        return Err("Received file count did not match the offer".into());
    }
    let received_directory = destination.join("native-smoke");
    let mut hashes = Vec::new();
    for (index, ((path, expected), bytes)) in received.iter().zip(&files).zip(&content).enumerate()
    {
        let path = Path::new(path);
        if path.parent() != Some(received_directory.as_path()) {
            return Err("Received file escaped its destination".into());
        }
        if path.file_name().and_then(|name| name.to_str())
            != Some(format!("{}-{}", index + 1, expected.filename).as_str())
        {
            return Err("Received filename did not match the offer".into());
        }
        let actual = fs::read(path).map_err(|error| error.to_string())?;
        let hash = format!("{:x}", Sha256::digest(&actual));
        if actual != *bytes || hash != expected.sha256 {
            return Err("Received file bytes did not match".into());
        }
        hashes.push(json!({"filename": expected.filename, "bytes": actual.len(), "sha256": hash}));
    }
    if fs::read_dir(&received_directory)
        .map_err(|error| error.to_string())?
        .count()
        != files.len()
    {
        return Err("Incomplete temporary files remain after completion".into());
    }
    Ok(json!({"ok": true, "transport": "fips-tcp", "files": hashes,
        "bytes_before_accept": 0, "total_bytes": content.iter().map(Vec::len).sum::<usize>(),
        "sender_completed": true, "receiver_completed": true, "public_storage_used": false}))
}
