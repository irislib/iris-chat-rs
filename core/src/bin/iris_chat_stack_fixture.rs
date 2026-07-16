use std::io::{self, BufRead};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::Engine;
use iris_chat_core::{download_hashtree_attachment, AppAction, FfiApp};
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
    emit(json!({
        "event": "ready",
        "npub": account.device_npub,
        "owner_npub": account.npub,
    }))?;

    for line in io::stdin().lock().lines() {
        let line = line.context("read Chat fixture command")?;
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some("fetch"), Some(nhash), None) => emit(fetch_event(nhash))?,
            (Some("status"), None, None) => emit(json!({
                "event": "status",
                "npub": account.device_npub,
                "owner_npub": account.npub,
            }))?,
            (Some("stop"), None, None) => {
                app.shutdown();
                emit(json!({ "event": "stopped" }))?;
                return Ok(());
            }
            _ => emit(json!({
                "event": "error",
                "error": "expected fetch <nhash>, status, or stop",
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
