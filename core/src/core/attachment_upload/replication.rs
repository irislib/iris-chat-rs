use super::*;
use nostr::{EventBuilder, JsonUtil, Kind, Tag, TagKind};

pub(super) fn bounded_servers(servers: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for server in servers {
        if server.len() > 512 {
            continue;
        }
        let Ok(url) = url::Url::parse(server) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            continue;
        }
        let server = url.as_str().trim_end_matches('/').to_string();
        if !result.contains(&server) {
            result.push(server);
        }
        if result.len() == local_store::MAX_SERVERS {
            break;
        }
    }
    result
}

/// A receipt is historical: this server acknowledged all blocks and served this
/// exact root ciphertext URL. It is never an assertion of present availability.
async fn upload_block(
    client: &reqwest::Client,
    keys: &nostr::Keys,
    server: &str,
    hash: &Hash,
    data: &[u8],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        hashtree_core::sha256(data) == *hash,
        "attachment hash mismatch"
    );
    let hash = to_hex(hash);
    let auth = EventBuilder::new(Kind::Custom(24242), "Upload")
        .tags([
            Tag::custom(TagKind::custom("t"), ["upload"]),
            Tag::custom(TagKind::custom("x"), [hash.as_str()]),
            Tag::custom(
                TagKind::custom("expiration"),
                [(local_store::now() + 300).to_string()],
            ),
        ])
        .sign_with_keys(keys)?;
    let mut response = client
        .put(format!("{server}/upload"))
        .header(
            "Authorization",
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD.encode(auth.as_json())
            ),
        )
        .header("Content-Type", "application/octet-stream")
        .header("x-sha-256", &hash)
        .body(data.to_vec())
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "attachment server rejected upload"
    );
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= 4096),
        "upload acknowledgment is too large"
    );
    let mut body = Vec::new();
    while let Some(part) = response.chunk().await? {
        anyhow::ensure!(
            body.len() + part.len() <= 4096,
            "upload acknowledgment is too large"
        );
        body.extend_from_slice(&part);
    }
    let descriptor = serde_json::from_slice::<serde_json::Value>(&body).ok();
    if let Some(remote_hash) = descriptor
        .as_ref()
        .and_then(|v| v.get("sha256"))
        .and_then(|v| v.as_str())
    {
        anyhow::ensure!(
            remote_hash == hash,
            "server acknowledged a different attachment hash"
        );
    } else {
        // Some supported Blossom servers return an empty 2xx. Verify the exact
        // hash-addressed ciphertext before treating that response as an ACK.
        anyhow::ensure!(
            download_attachment_blob(client, server, &hash, MAX_ATTACHMENT_BYTES + 64)
                .await?
                .is_some(),
            "server did not confirm the uploaded ciphertext"
        );
    }
    Ok(())
}

pub(super) async fn replicate_tree(
    store: &Arc<AttachmentChunkStore>,
    client: &reqwest::Client,
    keys: &nostr::Keys,
    root: Hash,
    server: &str,
) -> anyhow::Result<()> {
    let lease = store.lease(&root)?;
    let recovery = store.confirmed_servers(&root)?;
    for hash in &lease.hashes {
        let mut data = store.get(hash).await.ok().flatten();
        if data.is_none() {
            for previous in &recovery {
                if let Ok(Some(bytes)) = download_attachment_blob(
                    client,
                    previous,
                    &to_hex(hash),
                    MAX_ATTACHMENT_BYTES + 64,
                )
                .await
                {
                    store
                        .put(*hash, bytes.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    data = Some(bytes);
                    break;
                }
            }
        }
        let data = data.ok_or_else(|| anyhow::anyhow!("attachment block is unavailable"))?;
        upload_block(client, keys, server, hash, &data).await?;
    }
    // This is the actual download route used by the app, not a response URL
    // that could contain credentials or a key, or point at another origin.
    anyhow::ensure!(
        download_attachment_blob(client, server, &to_hex(&root), MAX_ATTACHMENT_BYTES + 64)
            .await?
            .is_some(),
        "root ciphertext URL is unavailable"
    );
    store.finish_replica(&root, server, true)?;
    Ok(())
}

pub(in crate::core) struct AttachmentService {
    pub(in crate::core) store: Arc<AttachmentChunkStore>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for AttachmentService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl AppCore {
    pub(in crate::core) fn ensure_attachment_service(
        &mut self,
    ) -> anyhow::Result<Arc<AttachmentChunkStore>> {
        if let Some(service) = &self.attachment_service {
            return Ok(service.store.clone());
        }
        let account = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no active attachment account"))?;
        let keys = account
            .owner_keys
            .as_ref()
            .unwrap_or(&account.device_keys)
            .clone();
        let store = AttachmentChunkStore::open(
            &self.data_dir.join("attachments"),
            local_store::DEFAULT_BUDGET,
        )?;
        store.configure_pending(&blossom_servers_from_config().1)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let worker_store = store.clone();
        let sender = self.core_sender.clone();
        let task = self.runtime.spawn(async move {
            loop {
                match worker_store.next_replica() {
                    Ok(Some((root, server))) => {
                        let complete = tokio::time::timeout(
                            Duration::from_secs(600),
                            replicate_tree(&worker_store, &client, &keys, root, &server),
                        )
                        .await
                        .is_ok_and(|result| result.is_ok());
                        if complete {
                            let _ = sender.send(CoreMsg::Internal(Box::new(
                                InternalEvent::AttachmentReceiptsUpdated,
                            )));
                        } else if worker_store.finish_replica(&root, &server, false).is_err() {
                            break;
                        }
                    }
                    Ok(None) => tokio::select! {
                        _ = worker_store.wake.notified() => (),
                        _ = tokio::time::sleep(Duration::from_secs(30)) => (),
                    },
                    Err(_) => break,
                }
            }
        });
        *active_local_store()
            .write()
            .unwrap_or_else(|p| p.into_inner()) = Some(Arc::downgrade(&store));
        self.attachment_service = Some(AttachmentService {
            store: store.clone(),
            task,
        });
        Ok(store)
    }

    pub(in crate::core) fn clear_attachment_service(&mut self) {
        if let Some(service) = self.attachment_service.take() {
            service.task.abort();
            let mut active = active_local_store()
                .write()
                .unwrap_or_else(|p| p.into_inner());
            if active
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|store| store.id == service.store.id)
            {
                *active = None;
            }
            drop(active);
            if let Err(error) = service.store.close_and_clear() {
                self.push_debug_log("attachment.storage.clear.error", error.to_string());
            }
        }
    }
}
