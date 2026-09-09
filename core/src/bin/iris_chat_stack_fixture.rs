use std::io::{self, BufRead};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::Engine;
use iris_chat_core::stack_mesh_fixture::stack_mesh_fixture;
use iris_chat_core::{download_hashtree_attachment, AppAction, FfiApp};
use nostr::EventId;
use nostr_pubsub::Filter;
use serde_json::json;
use sha2::{Digest, Sha256};

const READY_WAIT: Duration = Duration::from_secs(15);

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let command = args
        .next()
        .context("usage: iris-chat-stack-fixture run <data-dir>")?;
    if command != "run" {
        bail!("usage: iris-chat-stack-fixture run <data-dir>");
    }
    let data_dir = PathBuf::from(
        args.next()
            .context("usage: iris-chat-stack-fixture run <data-dir>")?,
    );
    if args.next().is_some() {
        bail!("usage: iris-chat-stack-fixture run <data-dir>");
    }
    std::fs::create_dir_all(&data_dir).context("create Chat fixture data directory")?;

    let app = FfiApp::new(
        data_dir.to_string_lossy().into_owned(),
        String::new(),
        String::new(),
    );
    app.dispatch(AppAction::CreateAccount {
        name: "Iris Stack fixture".to_string(),
    });
    let deadline = Instant::now() + READY_WAIT;
    let account = loop {
        let state = app.state();
        if let Some(account) = state.account {
            break account;
        }
        if let Some(error) = state.toast {
            bail!("Chat fixture account setup failed: {error}");
        }
        if Instant::now() >= deadline {
            bail!("Chat fixture account setup timed out");
        }
        thread::sleep(Duration::from_millis(25));
    };
    let mesh = if std::env::var("IRIS_CHAT_FIPS_ROUTED_PEERS").is_ok() {
        let deadline = Instant::now() + READY_WAIT;
        loop {
            if let Some(mesh) = stack_mesh_fixture() {
                break Some(mesh);
            }
            if Instant::now() >= deadline {
                bail!("Chat shared mesh runtime did not start");
            }
            thread::sleep(Duration::from_millis(25));
        }
    } else {
        stack_mesh_fixture()
    };
    let runtime = tokio::runtime::Runtime::new()?;
    let mut subscription = mesh
        .as_ref()
        .map(|mesh| runtime.block_on(mesh.client.subscribe(vec![Filter::new()])))
        .transpose()?;
    emit(json!({
        "event": "ready",
        "npub": account.device_npub,
        "owner_npub": account.npub,
    }))?;

    for line in io::stdin().lock().lines() {
        let line = line.context("read Chat fixture command")?;
        if let Some(command) = line.strip_prefix("publish ") {
            let mesh = mesh.as_ref().context("Chat mesh runtime unavailable")?;
            let (kind, content) = command
                .split_once(' ')
                .context("expected publish <kind> <content>")?;
            emit(runtime.block_on(mesh.publish(kind.parse()?, content))?)?;
            continue;
        }
        if let Some(id) = line.strip_prefix("receive ") {
            let id = EventId::parse(id.trim())?;
            let subscription = subscription
                .as_mut()
                .context("Chat mesh subscription unavailable")?;
            let received = runtime.block_on(async {
                tokio::time::timeout(READY_WAIT, async {
                    while let Some(delivery) = subscription.recv().await {
                        if delivery.event.as_event().id == id {
                            return Some(delivery);
                        }
                    }
                    None
                })
                .await
            });
            emit(match received {
                Ok(Some(delivery)) => {
                    let event = delivery.event.into_event();
                    json!({"event": "received", "id": event.id.to_string(),
                        "pubkey": event.pubkey.to_hex(), "kind": event.kind.as_u16(),
                        "content": event.content, "verified": event.verify().is_ok(),
                        "origin_peer_id": delivery.source.id.0})
                }
                _ => {
                    json!({"event": "received", "id": id.to_string(), "error": "event receive timed out or stopped"})
                }
            })?;
            continue;
        }
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some("fetch"), Some(nhash), None) => emit(fetch_event(nhash))?,
            (Some("status"), None, None) => emit(match &mesh {
                Some(mesh) => runtime.block_on(mesh.status())?,
                None => json!({"event": "status", "npub": account.device_npub,
                    "owner_npub": account.npub}),
            })?,
            (Some("stop"), None, None) => {
                app.shutdown();
                emit(json!({ "event": "stopped" }))?;
                return Ok(());
            }
            _ => emit(json!({
                "event": "error",
                "error": "expected fetch <nhash>, publish <kind> <content>, receive <id>, status, or stop",
            }))?,
        }
    }
    app.shutdown();
    Ok(())
}

fn fetch_event(nhash: &str) -> serde_json::Value {
    let result = download_hashtree_attachment(nhash.to_string());
    match result.data_base64 {
        Some(encoded) => match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(bytes) => json!({
                "event": "fetch",
                "nhash": nhash,
                "fetched": bytes.len(),
                "sha256": format!("{:x}", Sha256::digest(&bytes)),
            }),
            Err(error) => json!({
                "event": "fetch",
                "nhash": nhash,
                "fetched": 0,
                "error": format!("invalid production base64: {error}"),
            }),
        },
        None => json!({
            "event": "fetch",
            "nhash": nhash,
            "fetched": 0,
            "error": result.error.unwrap_or_else(|| "attachment fetch failed".to_string()),
        }),
    }
}

fn emit(value: serde_json::Value) -> Result<()> {
    println!("{value}");
    use std::io::Write;
    io::stdout().flush().context("flush Chat fixture event")
}
