//! Encrypted chat messages carry signed offers; file bytes use the dedicated FIPS stream.
use super::direct_file_tcp::{DirectFileEvent, DirectFileSender, TransferFile};
use super::*;
use crate::{DirectFileSnapshot, DirectFileTransferSnapshot, DirectFileTransferStatus};
use fips_core::{FipsEndpoint, PeerIdentity as FipsPeerIdentity};
mod model;
mod prepare;
mod storage;
#[cfg(test)]
mod tests;
pub(crate) use model::Record;
use model::*;

pub(super) enum Action {
    Accept,
    Decline,
    Cancel,
}

impl AppCore {
    pub(super) fn send_direct_files(
        &mut self,
        chat: &str,
        attachments: Vec<OutgoingAttachment>,
        caption: String,
    ) {
        let result = self.begin_direct_files(chat, attachments, caption);
        if let Err(error) = result {
            self.state.toast = Some(error);
        }
        self.emit_state();
    }

    fn begin_direct_files(
        &mut self,
        chat: &str,
        attachments: Vec<OutgoingAttachment>,
        caption: String,
    ) -> Result<(), String> {
        let chat_id = self.normalize_chat_id(chat).ok_or("Invalid chat.")?;
        if !self.can_use_chats() || is_group_chat_id(&chat_id) || self.is_owner_blocked(&chat_id) {
            return Err("Direct files are available in a private chat.".into());
        }
        if attachments.is_empty() || attachments.len() > MAX_FILES {
            return Err("Choose between 1 and 32 files.".into());
        }
        if caption.len() > 4096 {
            return Err("The caption is too long.".into());
        }
        if self.state.busy.sending_message {
            return Err("Please wait for the current message.".into());
        }
        if self.device_sync.is_none() {
            self.reconcile_device_sync();
        }
        self.direct_sender()?;
        let account = self.logged_in.as_ref().ok_or("Sign in first.")?;
        let device = account.device_keys.public_key().to_hex();
        if self.direct_allowed_peers(&chat_id, &device).is_empty() {
            return Err("No other device is available to receive these files.".into());
        }
        let id = hex_random::<16>();
        let record = Record {
            chat_id: chat_id.clone(),
            wire: String::new(),
            offer: Offer {
                id: id.clone(),
                token: hex_random::<32>(),
                owner: account.owner_pubkey.to_hex(),
                recipient: chat_id,
                device: device.clone(),
                caption: caption.trim().into(),
                expires_at_secs: unix_now().get().saturating_add(24 * 60 * 60),
                files: Vec::new(),
            },
            is_sender: true,
            status: DirectFileTransferStatus::Offered,
            paths: Vec::new(),
            peer: None,
            transferred: 0,
            error: None,
        };
        let directory = self.data_dir.join("direct-files").join(&id).join("send");
        let keys = account.device_keys.clone();
        let sender = self.core_sender.clone();
        let generation = self.fips_connection_generation;
        self.state.busy.sending_message = true;
        self.runtime.spawn_blocking(move || {
            let result = prepare::prepare(directory.clone(), attachments, record, keys);
            if sender
                .send(CoreMsg::Internal(Box::new(
                    InternalEvent::DirectFilesPrepared {
                        generation,
                        device,
                        result,
                    },
                )))
                .is_err()
            {
                let _ = std::fs::remove_dir_all(directory);
            }
        });
        Ok(())
    }

    pub(super) fn direct_files_prepared(
        &mut self,
        generation: u64,
        device: String,
        result: Result<Record, String>,
    ) {
        if generation != self.fips_connection_generation
            || self
                .logged_in
                .as_ref()
                .is_none_or(|a| a.device_keys.public_key().to_hex() != device)
        {
            if let Ok(record) = result {
                self.remove_direct_sources(&record);
            }
            return;
        }
        self.state.busy.sending_message = false;
        let result = result.and_then(|mut record| {
            let outcome = self.publish_direct_offer(&record);
            if let Err(error) = &outcome {
                if let Ok(tx) = self.direct_sender() {
                    let _ = tx.cancel(&record.offer.id);
                }
                record.status = DirectFileTransferStatus::Failed;
                record.error = Some(error.clone());
                self.remove_direct_sources(&record);
                record.paths.clear();
                let _ = storage::save(&self.app_store.shared(), &record);
            }
            outcome
        });
        if let Err(error) = result {
            self.state.toast = Some(error);
        }
        self.rebuild_state();
        self.emit_state();
    }

    fn publish_direct_offer(&mut self, record: &Record) -> Result<(), String> {
        if !self.can_use_chats() || self.is_owner_blocked(&record.chat_id) {
            return Err("The chat is unavailable.".into());
        }
        let tx = self.direct_sender()?;
        let peers = self.direct_allowed_peers(&record.offer.recipient, &record.offer.device);
        storage::save(&self.app_store.shared(), record)?;
        tx.register_offer(
            record.offer.id.clone(),
            record.offer.token.clone(),
            peers,
            record.offer.transport_files(&record.paths),
        )?;
        self.send_message(&record.chat_id, &record.wire, None);
        if !self.threads.get(&record.chat_id).is_some_and(|t| {
            t.messages
                .iter()
                .any(|m| m.body == record.wire && m.delivery != DeliveryState::Failed)
        }) {
            return Err("The file offer could not be sent.".into());
        }
        Ok(())
    }

    fn direct_sender(&self) -> Result<DirectFileSender, String> {
        self.device_sync
            .as_ref()
            .and_then(|r| r.direct_files.clone())
            .ok_or_else(|| "File transfer is not ready. Try again.".into())
    }

    fn direct_allowed_peers(&self, owner: &str, source: &str) -> Vec<FipsPeerIdentity> {
        self.app_keys
            .get(owner)
            .into_iter()
            .flat_map(|r| &r.devices)
            .filter(|d| d.identity_pubkey_hex != source)
            .filter_map(|d| fips_peer_from_hex(&d.identity_pubkey_hex))
            .collect()
    }

    fn direct_offer(&self, chat: &str, id: &str) -> Result<Record, String> {
        if let Some(record) = storage::load(&self.app_store.shared(), id)? {
            if record.chat_id != chat {
                return Err("File offer not found.".into());
            }
            return Ok(record);
        }
        let account = self.logged_in.as_ref().ok_or("Sign in first.")?;
        let owner = account.owner_pubkey.to_hex();
        let mut candidates: Vec<ChatMessageSnapshot> = self
            .threads
            .get(chat)
            .map(|t| t.messages.clone())
            .unwrap_or_default();
        if !candidates
            .iter()
            .any(|m| parse(&m.body).is_some_and(|o| o.id == id))
        {
            let db = self.app_store.shared();
            let conn = db.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare("SELECT body, author_owner_pubkey_hex FROM messages WHERE chat_id=?1 AND body LIKE 'iris-direct-file-v1:%'").map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([chat], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (body, author) = row.map_err(|e| e.to_string())?;
                if let Some(offer) =
                    parse(&body).filter(|o| o.id == id && author.as_ref() == Some(&o.owner))
                {
                    return self.record_for_received_offer(chat, offer, body);
                }
            }
        }
        for message in candidates.drain(..) {
            if let Some(offer) = parse(&message.body).filter(|o| o.id == id) {
                let author = message
                    .author_owner_pubkey_hex
                    .as_deref()
                    .unwrap_or(if message.is_outgoing { &owner } else { chat });
                if author == offer.owner {
                    return self.record_for_received_offer(chat, offer, message.body);
                }
            }
        }
        Err("File offer not found.".into())
    }

    fn record_for_received_offer(
        &self,
        chat: &str,
        offer: Offer,
        wire: String,
    ) -> Result<Record, String> {
        if offer.expires_at_secs <= unix_now().get() {
            return Err("This file offer has expired.".into());
        }
        let account = self.logged_in.as_ref().ok_or("Sign in first.")?;
        let owner = account.owner_pubkey.to_hex();
        if offer.recipient != owner
            || offer.owner != chat
            || offer.device == account.device_keys.public_key().to_hex()
        {
            return Err("Open this offer on the receiving device.".into());
        }
        if !self.app_keys.get(&offer.owner).is_some_and(|roster| {
            roster
                .devices
                .iter()
                .any(|d| d.identity_pubkey_hex == offer.device)
        }) {
            return Err("The sending device is no longer available.".into());
        }
        Ok(Record {
            chat_id: chat.into(),
            wire,
            offer,
            is_sender: false,
            status: DirectFileTransferStatus::Offered,
            paths: vec![],
            peer: None,
            transferred: 0,
            error: None,
        })
    }

    pub(super) fn act_on_direct_files(&mut self, chat: &str, id: &str, action: Action) {
        let result = (|| {
            if !self.can_use_chats() || self.is_owner_blocked(chat) {
                return Err("The chat is unavailable.".into());
            }
            let mut record = self.direct_offer(chat, id)?;
            if !active(&record.status) {
                return Err("This file offer has ended.".into());
            }
            if self.device_sync.is_none() {
                self.reconcile_device_sync();
            }
            let tx = self.direct_sender()?;
            match action {
                Action::Accept | Action::Decline => {
                    if record.is_sender || record.status != DirectFileTransferStatus::Offered {
                        return Err("Open this offer on the receiving device.".into());
                    }
                    self.record_for_received_offer(
                        chat,
                        record.offer.clone(),
                        record.wire.clone(),
                    )?;
                    let peer = fips_peer_from_hex(&record.offer.device)
                        .ok_or("Invalid sending device.")?;
                    if matches!(action, Action::Accept) {
                        record.status = DirectFileTransferStatus::Connecting;
                        storage::save(&self.app_store.shared(), &record)?;
                        if let Err(error) = tx.receive(
                            id.into(),
                            record.offer.token.clone(),
                            peer,
                            record.offer.transport_files(&[]),
                            self.data_dir.join("direct-files").join(id).join("received"),
                        ) {
                            record.status = DirectFileTransferStatus::Failed;
                            record.error = Some(error.clone());
                            storage::save(&self.app_store.shared(), &record)?;
                            return Err(error);
                        }
                    } else {
                        tx.decline(id.into(), record.offer.token.clone(), peer)?;
                        record.status = DirectFileTransferStatus::Declined;
                    }
                }
                Action::Cancel => {
                    tx.cancel(id)?;
                    record.status = DirectFileTransferStatus::Cancelled;
                }
            }
            self.cleanup_direct_sources(&mut record);
            storage::save(&self.app_store.shared(), &record)?;
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.state.toast = Some(error);
        }
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn handle_direct_file_event(&mut self, generation: u64, event: DirectFileEvent) {
        if generation != self.fips_connection_generation {
            return;
        }
        let (id, peer) = match &event {
            DirectFileEvent::Accepted { transfer_id, peer }
            | DirectFileEvent::Progress {
                transfer_id, peer, ..
            }
            | DirectFileEvent::Completed {
                transfer_id, peer, ..
            }
            | DirectFileEvent::Declined { transfer_id, peer }
            | DirectFileEvent::Cancelled { transfer_id, peer }
            | DirectFileEvent::Failed {
                transfer_id, peer, ..
            } => (transfer_id, peer),
        };
        let Ok(Some(mut record)) = storage::load(&self.app_store.shared(), id) else {
            return;
        };
        if !active(&record.status) {
            return;
        }
        let local_failure = peer.is_empty()
            && matches!(
                event,
                DirectFileEvent::Failed { .. } | DirectFileEvent::Cancelled { .. }
            );
        let allowed = if local_failure {
            true
        } else if record.is_sender {
            self.app_keys.get(&record.offer.recipient).is_some_and(|r| {
                r.devices
                    .iter()
                    .any(|d| &d.identity_pubkey_hex == peer && peer != &record.offer.device)
            })
        } else {
            peer == &record.offer.device
        };
        if !allowed || (!local_failure && record.peer.as_ref().is_some_and(|p| p != peer)) {
            if let Ok(tx) = self.direct_sender() {
                let _ = tx.cancel(id);
            }
            record.status = DirectFileTransferStatus::Failed;
            record.error = Some("The receiving device is no longer available.".into());
        } else {
            if !peer.is_empty() {
                record.peer = Some(peer.clone());
            }
            match event {
                DirectFileEvent::Accepted { .. } => {
                    record.status = DirectFileTransferStatus::Transferring
                }
                DirectFileEvent::Progress {
                    transferred_bytes,
                    total_bytes,
                    ..
                } => {
                    debug_assert_eq!(total_bytes, record.offer.total());
                    record.status = DirectFileTransferStatus::Transferring;
                    record.transferred = transferred_bytes.min(record.offer.total());
                }
                DirectFileEvent::Completed { local_paths, .. } => {
                    record.status = DirectFileTransferStatus::Completed;
                    record.transferred = record.offer.total();
                    if !record.is_sender {
                        record.paths = local_paths;
                    }
                }
                DirectFileEvent::Declined { .. } => {
                    record.status = DirectFileTransferStatus::Declined
                }
                DirectFileEvent::Cancelled { .. } => {
                    record.status = DirectFileTransferStatus::Cancelled
                }
                DirectFileEvent::Failed { error, .. } => {
                    record.status = DirectFileTransferStatus::Failed;
                    record.error = Some(error);
                }
            }
        }
        self.cleanup_direct_sources(&mut record);
        if let Err(error) = storage::save(&self.app_store.shared(), &record) {
            self.state.toast = Some(error);
        }
        self.rebuild_state();
        self.emit_state();
    }

    fn remove_direct_sources(&self, record: &Record) {
        let directory = self
            .data_dir
            .join("direct-files")
            .join(&record.offer.id)
            .join("send");
        let _ = std::fs::remove_dir_all(directory);
    }

    fn cleanup_direct_sources(&self, record: &mut Record) {
        if record.is_sender
            && !active(&record.status)
            && record.status != DirectFileTransferStatus::Completed
        {
            self.remove_direct_sources(record);
            record.paths.clear();
        }
    }

    pub(super) fn restrict_direct_file_devices(&mut self) {
        let Ok(tx) = self.direct_sender() else {
            return;
        };
        let Ok(records) = storage::all(&self.app_store.shared()) else {
            return;
        };
        for record in records.into_iter().filter(|r| active(&r.status)) {
            if record.is_sender {
                let peers =
                    self.direct_allowed_peers(&record.offer.recipient, &record.offer.device);
                let _ = tx.restrict_offer(&record.offer.id, peers);
            } else if !self.app_keys.get(&record.offer.owner).is_some_and(|r| {
                r.devices
                    .iter()
                    .any(|d| d.identity_pubkey_hex == record.offer.device)
            }) {
                let _ = tx.cancel(&record.offer.id);
            }
        }
    }

    pub(super) fn cancel_direct_files_for_chat(&mut self, chat: &str) {
        let Ok(records) = storage::all(&self.app_store.shared()) else {
            return;
        };
        for mut record in records
            .into_iter()
            .filter(|r| r.chat_id == chat && active(&r.status))
        {
            if let Ok(tx) = self.direct_sender() {
                if record.is_sender {
                    let _ = tx.restrict_offer(&record.offer.id, Vec::new());
                }
                let _ = tx.cancel(&record.offer.id);
            }
            record.status = DirectFileTransferStatus::Cancelled;
            self.cleanup_direct_sources(&mut record);
            let _ = storage::save(&self.app_store.shared(), &record);
        }
    }

    pub(super) fn interrupt_direct_files(&mut self) {
        if let Err(error) = storage::interrupt(&self.app_store.shared()) {
            self.push_debug_log("direct_files.interrupt", error);
        }
        if let Ok(records) = storage::all(&self.app_store.shared()) {
            for mut record in records {
                self.cleanup_direct_sources(&mut record);
                let _ = storage::save(&self.app_store.shared(), &record);
            }
        }
        self.state.busy.sending_message = false;
    }

    pub(super) fn start_direct_file_transport(
        &mut self,
        endpoint: Arc<FipsEndpoint>,
        tasks: &mut Vec<tokio::task::JoinHandle<()>>,
    ) -> Option<DirectFileSender> {
        let (event_tx, event_rx) = flume::bounded(64);
        match self
            .runtime
            .block_on(super::direct_file_tcp::start_direct_file_tcp(
                endpoint, event_tx,
            )) {
            Ok((tx, task)) => {
                tasks.push(task);
                let sender = self.core_sender.clone();
                let generation = self.fips_connection_generation;
                tasks.push(self.runtime.spawn(async move {
                    while let Ok(event) = event_rx.recv_async().await {
                        if sender
                            .send_async(CoreMsg::Internal(Box::new(InternalEvent::DirectFile {
                                generation,
                                event,
                            })))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
                Some(tx)
            }
            Err(error) => {
                self.push_debug_log("direct_files.start", error);
                None
            }
        }
    }
}

fn hex_random<const N: usize>() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(super) fn preview(body: &str) -> Option<String> {
    if !body.starts_with(PREFIX) {
        return None;
    }
    Some(
        parse(body)
            .map(|o| {
                if o.caption.is_empty() {
                    format!(
                        "{} direct file{}",
                        o.files.len(),
                        if o.files.len() == 1 { "" } else { "s" }
                    )
                } else {
                    o.caption
                }
            })
            .unwrap_or_else(|| "File offer".into()),
    )
}

/// A self-addressed offer can reach our linked devices before relay publication.
/// It is still device-signed metadata; accepting it never synchronizes file bytes.
pub(super) fn is_pending_self_offer(
    body: &str,
    chat_id: &str,
    author: Option<&str>,
    outgoing: bool,
) -> bool {
    outgoing
        && author == Some(chat_id)
        && parse(body).is_some_and(|offer| {
            offer.owner == chat_id
                && offer.recipient == chat_id
                && offer.expires_at_secs > unix_now().get()
        })
}

pub(super) fn decorate(
    message: &mut ChatMessageSnapshot,
    account: Option<&AccountSnapshot>,
    db: &SharedConnection,
) {
    if !message.body.starts_with(PREFIX) {
        return;
    }
    let Some(offer) = parse(&message.body) else {
        message.body = "File offer unavailable".into();
        return;
    };
    message.body = offer.caption.clone();
    let Some(account) = account else {
        return;
    };
    let author = message
        .author_owner_pubkey_hex
        .as_deref()
        .unwrap_or(if message.is_outgoing {
            &account.public_key_hex
        } else {
            &message.chat_id
        });
    if author != offer.owner {
        message.body = "File offer unavailable".into();
        return;
    }
    let record = storage::load(db, &offer.id).ok().flatten().filter(|r| {
        r.chat_id == message.chat_id
            && r.offer.device == offer.device
            && r.offer.token == offer.token
    });
    let is_sender = offer.device == account.device_public_key_hex;
    let status = record.as_ref().map(|r| r.status.clone()).unwrap_or(
        if is_sender
            || offer.recipient != account.public_key_hex
            || offer.expires_at_secs <= unix_now().get()
        {
            DirectFileTransferStatus::Unavailable
        } else {
            DirectFileTransferStatus::Offered
        },
    );
    message.direct_transfer = Some(DirectFileTransferSnapshot {
        id: offer.id.clone(),
        files: offer
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| DirectFileSnapshot {
                filename: f.filename.clone(),
                size_bytes: f.size_bytes,
                local_path: record
                    .as_ref()
                    .filter(|r| r.status == DirectFileTransferStatus::Completed)
                    .and_then(|r| r.paths.get(i))
                    .filter(|path| {
                        std::fs::metadata(path).is_ok_and(|metadata| {
                            metadata.is_file() && metadata.len() == f.size_bytes
                        })
                    })
                    .cloned(),
            })
            .collect(),
        status,
        is_sender,
        transferred_bytes: record.as_ref().map(|r| r.transferred).unwrap_or(0),
        total_bytes: offer.total(),
        error: record.and_then(|r| r.error),
    });
}

pub(super) fn interrupt_stored(db: &SharedConnection) {
    let _ = storage::interrupt(db);
}
