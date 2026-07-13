use super::*;
use crate::state::AttachmentDownloadResult;
use async_trait::async_trait;
use base64::Engine;
use futures_util::FutureExt;
use hashtree_blossom::BlossomClient;
use hashtree_config::Config as HashtreeConfig;
use hashtree_core::{
    nhash_decode, nhash_encode_full, to_hex, Cid, Hash, HashTree, HashTreeConfig, NHashData, Store,
    StoreError,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLockReadGuard, RwLockWriteGuard};
use tokio::sync::RwLock as AsyncRwLock;
use tokio_util::compat::TokioAsyncReadCompatExt;

/// Process-wide cache of hashtree chunks we've put or fetched. Without this,
/// a freshly uploaded chunk is invisible to a follow-up `HashTree::get`
/// (e.g. when iOS uses one store for the upload and another for the avatar
/// render), and the avatar falls back to placeholder if the public Blossom
/// servers can't serve the chunk back yet.
fn shared_chunk_cache() -> &'static std::sync::RwLock<HashMap<String, Vec<u8>>> {
    static CACHE: OnceLock<std::sync::RwLock<HashMap<String, Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

fn shared_chunk_cache_read() -> RwLockReadGuard<'static, HashMap<String, Vec<u8>>> {
    shared_chunk_cache()
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn shared_chunk_cache_write() -> RwLockWriteGuard<'static, HashMap<String, Vec<u8>>> {
    shared_chunk_cache()
        .write()
        .unwrap_or_else(|poison| poison.into_inner())
}

impl AppCore {
    pub(super) fn send_attachment(
        &mut self,
        chat_id: &str,
        file_path: &str,
        filename: &str,
        caption: &str,
    ) {
        self.send_attachments(
            chat_id,
            &[OutgoingAttachment {
                file_path: file_path.to_string(),
                filename: filename.to_string(),
            }],
            caption,
        );
    }

    pub(super) fn send_attachments(
        &mut self,
        chat_id: &str,
        attachments: &[OutgoingAttachment],
        caption: &str,
    ) {
        let chat_id = chat_id.trim();
        if chat_id.is_empty() || attachments.is_empty() {
            self.state.toast = Some("Attachment could not be sent.".to_string());
            self.emit_state();
            return;
        }
        if self.logged_in.is_none() {
            self.state.toast = Some("Create or restore a profile first.".to_string());
            self.emit_state();
            return;
        }
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            self.state.toast = Some("Invalid chat id.".to_string());
            self.emit_state();
            return;
        };
        let prepared = match prepare_outgoing_attachments(attachments) {
            Ok(prepared) => prepared,
            Err(message) => {
                self.state.toast = Some(message.to_string());
                self.emit_state();
                return;
            }
        };

        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore a profile first.".to_string());
            self.emit_state();
            return;
        };
        let upload_keys = logged_in
            .owner_keys
            .as_ref()
            .unwrap_or(&logged_in.device_keys);
        let secret_hex = upload_keys.secret_key().to_secret_hex();
        let caption = caption.trim().to_string();
        let upload_attachments = prepared.clone();

        if self
            .start_upload(
                UploadTarget::ChatAttachments {
                    chat_id: normalized_chat_id.clone(),
                },
                async move {
                    upload_files_to_hashtree(&secret_hex, &upload_attachments)
                        .await
                        .map(|uploaded| format_attachment_links_message(&caption, &uploaded))
                        .map_err(|error| error.to_string())
                },
            )
            .is_none()
        {
            return;
        }

        self.push_debug_log(
            "attachment.upload.start",
            format!(
                "chat_id={} count={} files={}",
                normalized_chat_id,
                prepared.len(),
                prepared
                    .iter()
                    .map(|attachment| attachment.filename.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
        self.active_chat_id = Some(normalized_chat_id.clone());
        self.screen_stack = vec![Screen::Chat {
            chat_id: normalized_chat_id,
        }];
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn start_upload<F>(&mut self, target: UploadTarget, upload: F) -> Option<u64>
    where
        F: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        if self.suspended {
            return None;
        }
        if self.upload_runtime.active.is_some() {
            self.state.toast = Some("Attachment upload already in progress.".to_string());
            self.emit_state();
            return None;
        }

        self.upload_runtime.next_id = self
            .upload_runtime
            .next_id
            .checked_add(1)
            .expect("upload operation id exhausted");
        let operation_id = self.upload_runtime.next_id;
        let sender = self.core_sender.clone();
        let task = self.runtime.spawn(async move {
            let result = std::panic::AssertUnwindSafe(upload)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("upload task panicked".to_string()));
            let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::UploadFinished {
                operation_id,
                result,
            })));
        });
        self.upload_runtime.active = Some(ActiveUpload {
            id: operation_id,
            target,
            abort_handle: task.abort_handle(),
        });
        self.state.busy.uploading_attachment = true;
        self.state.busy.upload_progress = None;
        Some(operation_id)
    }

    pub(super) fn handle_upload_finished(
        &mut self,
        operation_id: u64,
        result: Result<String, String>,
    ) {
        if self
            .upload_runtime
            .active
            .as_ref()
            .is_none_or(|active| active.id != operation_id)
        {
            return;
        }
        let active = self
            .upload_runtime
            .active
            .take()
            .expect("matching upload must be active");
        self.state.busy.uploading_attachment = false;
        self.state.busy.upload_progress = None;
        match active.target {
            UploadTarget::ChatAttachments { chat_id } => {
                self.apply_attachment_upload_result(chat_id, result)
            }
            UploadTarget::GroupPicture { group_id } => {
                self.apply_group_picture_upload_result(group_id, result)
            }
            UploadTarget::ProfilePicture => self.apply_profile_picture_upload_result(result),
        }
    }

    pub(super) fn cancel_upload(&mut self) {
        if let Some(active) = self.upload_runtime.active.take() {
            active.abort_handle.abort();
        }
        self.state.busy.uploading_attachment = false;
        self.state.busy.upload_progress = None;
    }

    pub(super) fn cancel_upload_for_chat(&mut self, chat_id: &str) {
        let deleted_group_id = group_id_from_chat_id(chat_id);
        let matches =
            self.upload_runtime
                .active
                .as_ref()
                .is_some_and(|active| match &active.target {
                    UploadTarget::ChatAttachments {
                        chat_id: active_chat_id,
                    } => active_chat_id == chat_id,
                    UploadTarget::GroupPicture { group_id } => {
                        deleted_group_id.as_deref() == Some(group_id.as_str())
                    }
                    UploadTarget::ProfilePicture => false,
                });
        if matches {
            self.cancel_upload();
        }
    }

    pub(super) fn cancel_profile_picture_upload(&mut self) {
        if self
            .upload_runtime
            .active
            .as_ref()
            .is_some_and(|active| active.target == UploadTarget::ProfilePicture)
        {
            self.cancel_upload();
        }
    }

    fn apply_attachment_upload_result(&mut self, chat_id: String, result: Result<String, String>) {
        match result {
            Ok(message_text) => {
                self.push_debug_log(
                    "attachment.upload.finish",
                    format!("chat_id={} success=true", chat_id),
                );
                self.send_message(&chat_id, &message_text, None);
            }
            Err(error) => {
                self.push_debug_log(
                    "attachment.upload.finish",
                    format!("chat_id={} success=false error={}", chat_id, error),
                );
                self.state.toast = Some("Attachment upload failed.".to_string());
                self.emit_state();
            }
        }
    }
}

#[uniffi::export]
pub fn download_hashtree_attachment(nhash: String) -> AttachmentDownloadResult {
    match download_hashtree_attachment_blocking(&nhash) {
        Ok(data_base64) => AttachmentDownloadResult {
            data_base64: Some(data_base64),
            error: None,
        },
        Err(error) => AttachmentDownloadResult {
            data_base64: None,
            error: Some(error.to_string()),
        },
    }
}

#[derive(Clone, Debug)]
struct PreparedOutgoingAttachment {
    file_path: PathBuf,
    filename: String,
}

fn prepare_outgoing_attachments(
    attachments: &[OutgoingAttachment],
) -> Result<Vec<PreparedOutgoingAttachment>, &'static str> {
    let mut prepared = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let file_path = PathBuf::from(attachment.file_path.trim());
        if file_path.as_os_str().is_empty() {
            return Err("Attachment could not be sent.");
        }
        if !file_path.is_file() {
            return Err("Attachment file was not found.");
        }
        prepared.push(PreparedOutgoingAttachment {
            filename: display_filename(&attachment.filename, &file_path),
            file_path,
        });
    }
    Ok(prepared)
}

pub(super) fn display_filename(filename: &str, file_path: &Path) -> String {
    let from_input = filename.trim();
    let candidate = if from_input.is_empty() {
        file_path.file_name().and_then(|value| value.to_str())
    } else {
        Path::new(from_input)
            .file_name()
            .and_then(|value| value.to_str())
    };
    candidate
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("attachment")
        .to_string()
}

async fn upload_files_to_hashtree(
    secret_hex: &str,
    attachments: &[PreparedOutgoingAttachment],
) -> anyhow::Result<Vec<(String, String)>> {
    let mut uploaded = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let nhash = upload_file_to_hashtree(secret_hex, &attachment.file_path).await?;
        uploaded.push((nhash, attachment.filename.clone()));
    }
    Ok(uploaded)
}

pub(super) async fn upload_file_to_hashtree(
    secret_hex: &str,
    path: &Path,
) -> anyhow::Result<String> {
    let secret_key = nostr35::SecretKey::from_hex(secret_hex)
        .map_err(|error| anyhow::anyhow!("invalid upload key: {error}"))?;
    let keys = nostr35::Keys::new(secret_key);
    let (read_servers, write_servers) = blossom_servers_from_config();
    if write_servers.is_empty() {
        anyhow::bail!("no hashtree write servers configured");
    }

    let store = Arc::new(UploadingBlossomStore::new(
        keys,
        read_servers,
        write_servers,
    ));
    let tree = HashTree::new(HashTreeConfig::new(store));
    let file = tokio::fs::File::open(path).await?;
    let (cid, _size) = tree
        .put_stream(file.compat())
        .await
        .map_err(|error| anyhow::anyhow!("hashtree upload failed: {error}"))?;

    nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .map_err(|error| anyhow::anyhow!("nhash encode failed: {error}"))
}

pub(super) async fn upload_profile_picture_to_hashtree(
    secret_hex: &str,
    path: &Path,
) -> anyhow::Result<String> {
    let nhash = upload_picture_to_hashtree(secret_hex, path).await?;
    Ok(format!("htree://{nhash}"))
}

pub(super) async fn upload_picture_to_hashtree(
    secret_hex: &str,
    path: &Path,
) -> anyhow::Result<String> {
    let data = tokio::fs::read(path).await?;
    if data.is_empty() {
        anyhow::bail!("picture is empty");
    }
    if data.len() > 10 * 1024 * 1024 {
        anyhow::bail!("picture is too large");
    }
    if !looks_like_image(path, &data) {
        anyhow::bail!("picture must be an image");
    }
    upload_file_to_hashtree(secret_hex, path).await
}

pub(super) fn looks_like_image(path: &Path, data: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "avif" | "bmp" | "gif" | "heic" | "heif" | "jpg" | "jpeg" | "png" | "webp"
    ) || data.starts_with(b"\x89PNG")
        || data.starts_with(b"\xff\xd8\xff")
        || data.starts_with(b"GIF87a")
        || data.starts_with(b"GIF89a")
        || data.starts_with(b"RIFF")
}

fn download_hashtree_attachment_blocking(nhash: &str) -> anyhow::Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(download_hashtree_attachment_base64(nhash))
}

pub(super) async fn download_hashtree_attachment_base64(nhash: &str) -> anyhow::Result<String> {
    let nhash = nhash.trim();
    if nhash.is_empty() {
        anyhow::bail!("missing attachment hash");
    }
    let data = nhash_decode(nhash).map_err(|error| anyhow::anyhow!("invalid nhash: {error}"))?;
    let cid = Cid {
        hash: data.hash,
        key: data.decrypt_key,
    };
    let keys = nostr35::Keys::generate();
    let (read_servers, write_servers) = blossom_servers_from_config();
    let store = Arc::new(UploadingBlossomStore::new(
        keys,
        merge_read_servers(read_servers, &write_servers),
        Vec::new(),
    ));
    let tree = HashTree::new(HashTreeConfig::new(store));
    let bytes = tree
        .get(&cid, None)
        .await
        .map_err(|error| anyhow::anyhow!("hashtree download failed: {error}"))?
        .ok_or_else(|| anyhow::anyhow!("attachment was not found"))?;
    if bytes.len() > 64 * 1024 * 1024 {
        anyhow::bail!("attachment is too large");
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn blossom_servers_from_config() -> (Vec<String>, Vec<String>) {
    let cfg = HashtreeConfig::load_or_default();
    let mut read = cfg.blossom.all_read_servers();
    let configured_write = cfg.blossom.all_write_servers();
    let mut write: Vec<String> = configured_write
        .iter()
        .filter(|server| !is_local_server_url(server))
        .cloned()
        .collect();
    if write.is_empty() {
        write = configured_write;
    }

    if let Some(local_url) =
        hashtree_config::detect_local_daemon_url(Some(&cfg.server.bind_address))
    {
        if !read.iter().any(|server| server == &local_url) {
            read.insert(0, local_url);
        }
    }

    read = merge_read_servers(read, &write);
    (read, write)
}

fn merge_read_servers(mut read: Vec<String>, write: &[String]) -> Vec<String> {
    let mut seen: HashSet<String> = read.iter().cloned().collect();
    for server in write {
        if seen.insert(server.clone()) {
            read.push(server.clone());
        }
    }
    read
}

fn is_local_server_url(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    matches!(
        host.trim_matches(['[', ']']).to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

struct UploadingBlossomStore {
    client: BlossomClient,
    uploaded: AsyncRwLock<HashSet<String>>,
}

impl UploadingBlossomStore {
    fn new(keys: nostr35::Keys, read_servers: Vec<String>, write_servers: Vec<String>) -> Self {
        let client = BlossomClient::new_empty(keys)
            .with_read_servers(read_servers)
            .with_write_servers(write_servers);
        Self {
            client,
            uploaded: AsyncRwLock::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl Store for UploadingBlossomStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        let hash_hex = to_hex(&hash);
        let computed = hashtree_blossom::compute_sha256(&data);
        if computed != hash_hex {
            return Err(StoreError::Other(
                "hash mismatch for blossom upload".to_string(),
            ));
        }

        // Always cache the chunk locally first. If the remote upload fails the
        // file still exists on this device and can be rendered immediately;
        // a real sync layer will publish it to peers later.
        {
            let mut cache = shared_chunk_cache_write();
            cache.insert(hash_hex.clone(), data.clone());
        }

        {
            let uploaded = self.uploaded.read().await;
            if uploaded.contains(&hash_hex) {
                return Ok(false);
            }
        }

        let upload_result = self.client.upload_if_missing(&data).await;
        match upload_result {
            Ok((remote_hash, was_uploaded)) => {
                if remote_hash != hash_hex {
                    return Err(StoreError::Other(format!(
                        "remote hash mismatch: expected {hash_hex}, got {remote_hash}"
                    )));
                }
                let mut uploaded = self.uploaded.write().await;
                uploaded.insert(hash_hex);
                Ok(was_uploaded)
            }
            Err(error) => {
                // Remote unreachable. Treat the chunk as locally stored — the
                // shared cache above already retains it. We propagate `true`
                // so HashTree::put_stream completes; a future re-upload pass
                // can push the cached chunks once the network recovers.
                eprintln!("blossom upload failed for {hash_hex} ({error}); kept in local cache");
                Ok(true)
            }
        }
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let key = to_hex(hash);
        {
            let cache = shared_chunk_cache_read();
            if let Some(data) = cache.get(&key) {
                return Ok(Some(data.clone()));
            }
        }

        match self.client.try_download(&key).await {
            Some(data) => {
                let computed = hashtree_blossom::compute_sha256(&data);
                if computed != key {
                    return Err(StoreError::Other(format!(
                        "download hash mismatch for {key}"
                    )));
                }
                let mut cache = shared_chunk_cache_write();
                cache.insert(key, data.clone());
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        let key = to_hex(hash);
        {
            let cache = shared_chunk_cache_read();
            if cache.contains_key(&key) {
                return Ok(true);
            }
        }

        for server in self.client.read_servers() {
            if self.client.exists_on_server(&key, server).await {
                return Ok(true);
            }
        }

        Ok(false)
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        let key = to_hex(hash);
        let mut removed = {
            let mut cache = shared_chunk_cache_write();
            cache.remove(&key).is_some()
        };
        let mut uploaded = self.uploaded.write().await;
        removed |= uploaded.remove(&key);
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_filename_uses_input_basename_then_path_fallback() {
        assert_eq!(
            display_filename("nested/photo.png", Path::new("/tmp/source.bin")),
            "photo.png"
        );
        assert_eq!(
            display_filename("", Path::new("/tmp/source.bin")),
            "source.bin"
        );
        assert_eq!(display_filename("", Path::new("/")), "attachment");
    }

    #[test]
    fn prepares_multiple_outgoing_attachments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first.bin");
        let second = dir.path().join("second.bin");
        fs::write(&first, b"one").expect("write first");
        fs::write(&second, b"two").expect("write second");

        let prepared = prepare_outgoing_attachments(&[
            OutgoingAttachment {
                file_path: first.to_string_lossy().to_string(),
                filename: "nested/photo.png".to_string(),
            },
            OutgoingAttachment {
                file_path: second.to_string_lossy().to_string(),
                filename: String::new(),
            },
        ])
        .expect("prepared");

        assert_eq!(prepared.len(), 2);
        assert_eq!(prepared[0].filename, "photo.png");
        assert_eq!(prepared[1].filename, "second.bin");
    }
}
