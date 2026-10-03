use super::*;
use fips_core::{FipsEndpoint, PeerIdentity as FipsPeerIdentity};
use nostr_double_ratchet::{GroupProtocol, GroupStrategy};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};
use nostr_pubsub_relay::RelayEventBus;
use std::collections::BTreeSet;
use tokio::task::JoinHandle;

use anti_entropy::metadata_page_packets;
use messages::collect_device_sync_messages;
use recent_peers::DeviceSyncRecentPeers;
use settings::valid_device_sync_chat_id;

mod anti_entropy;
mod body;
mod history;
mod history_policy;
mod messages;
mod recent_peers;
mod runtime;
mod settings;
#[cfg(test)]
mod test_support;

pub(super) const DEVICE_SYNC_PORT: u16 = 7369;
const DEVICE_SYNC_VERSION: u8 = 1;
const DEVICE_SYNC_MAX_PACKET_BYTES: usize = 64 * 1024;
const DEVICE_SYNC_PAGE_MESSAGES: usize = 32;
const DEVICE_SYNC_PAGE_PACKETS: usize = 32;
const DEVICE_SYNC_SCOPE_PREFIX: &str = "iris-chat-device-sync-v1:";
struct DeviceSyncConfig {
    key: String,
    owner_hex: String,
    local_npub: String,
    roster_at: u64,
    secret_hex: String,
    relay_urls: Vec<String>,
    relay_client: Option<Client>,
    siblings: Vec<FipsPeerIdentity>,
    peers: Vec<FipsPeerIdentity>,
    nearby_ip_enabled: bool,
}
pub(super) struct DeviceSyncRuntime {
    key: String,
    peer_refresh_key: String,
    pub(super) endpoint: Arc<FipsEndpoint>,
    pub(super) direct_files: Option<super::direct_file_tcp::DirectFileSender>,
    pub(super) calls_tx: Option<Sender<super::calls::MediaSend>>,
    tcp: Option<DeviceSyncTcpSender>,
    siblings: Vec<FipsPeerIdentity>,
    snapshot_pending: bool,
    history: history::HistoryState,
    pub(super) nearby_enabled: bool,
    pub(super) nearby_bootstrap_payloads: Arc<RwLock<Vec<Vec<u8>>>>,
    pub(super) nearby_outbox: Arc<RwLock<super::fips_nearby::FipsNearbyOutbox>>,
    _attachment_blobs: Option<Arc<super::attachment_upload::AttachmentBlobRuntime>>,
    pub(super) pubsub: Option<Arc<FipsPubsubClient>>,
    pub(super) protocol_subscriptions: super::mesh_pubsub::MeshProtocolSubscriptions,
    _update_provider: Option<Arc<dyn nostr_pubsub::NostrEventSubscriber>>,
    recent_peers: Option<Arc<RwLock<DeviceSyncRecentPeers>>>,
    tasks: Vec<JoinHandle<()>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum DeviceSyncPacket {
    Request {
        v: u8,
        roster_at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page: Option<DeviceSyncPage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history_reconcile: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history_since: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link_id: Option<String>,
    },
    ResyncRequired {
        v: u8,
    },
    PageEnd {
        v: u8,
        roster_at: u64,
        next: DeviceSyncPage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history_reconcile: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history_since: Option<u64>,
    },
    HistoryPageEnd {
        v: u8,
        link_at: u64,
        link_id: String,
    },
    HistoryPolicy {
        v: u8,
        link_at: u64,
        since: u64,
        link_id: String,
    },
    HistoryComplete {
        v: u8,
        link_at: u64,
        link_id: String,
    },
    HistoryOpen {
        v: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link_id: Option<String>,
        session: String,
        since: u64,
        until: u64,
        frame: String,
    },
    HistoryFrame {
        v: u8,
        session: String,
        frame: String,
    },
    HistoryNeed {
        v: u8,
        session: String,
        ids: Vec<String>,
    },
    HistoryMessages {
        v: u8,
        session: String,
        messages: Vec<DeviceSyncMessage>,
        requested: Vec<String>,
    },
    HistoryDone {
        v: u8,
        session: String,
    },
    Snapshot {
        v: u8,
        roster_at: u64,
        #[serde(default)]
        chats: Vec<DeviceSyncChat>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        deleted_chats: Vec<DeviceSyncChatDeletion>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        chat_mutes: Vec<ChatMuteState>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        chat_pins: Vec<ChatPinState>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        private_contacts_v2: Vec<crate::private_contact_sync_v2::PrivateContactDocumentV2>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        private_device_labels_v2: Vec<super::private_device_labels::PrivateDeviceLabel>,
        #[serde(default)]
        app_keys: Vec<DeviceSyncAppKeys>,
        #[serde(default)]
        groups: Vec<DeviceSyncGroup>,
        #[serde(default)]
        messages: Vec<DeviceSyncMessage>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncChat {
    id: String,
    updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    read_state: Option<ChatReadState>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncChatDeletion {
    id: String,
    deleted_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncAppKeys {
    owner_pubkey: String,
    created_at: u64,
    devices: Vec<DeviceSyncAppKeyDevice>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncAppKeyDevice {
    identity_pubkey: String,
    created_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncGroup {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    picture: Option<String>,
    created_by: String,
    members: Vec<String>,
    admins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    revision: u64,
    created_at: u64,
    updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncMessage {
    chat_id: String,
    id: String,
    #[serde(with = "body")]
    body: String,
    author: String,
    created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSyncCursor {
    created_at: u64,
    chat_id: String,
    id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum DeviceSyncPage {
    Metadata { offset: usize },
    Messages { after: Option<DeviceSyncCursor> },
}
impl From<&DeviceSyncMessage> for DeviceSyncCursor {
    fn from(message: &DeviceSyncMessage) -> Self {
        Self {
            created_at: message.created_at,
            chat_id: message.chat_id.clone(),
            id: message.id.clone(),
        }
    }
}
#[derive(Default)]
struct DeviceSyncSnapshot {
    roster_at: u64,
    chats: Vec<DeviceSyncChat>,
    deleted_chats: Vec<DeviceSyncChatDeletion>,
    chat_mutes: Vec<ChatMuteState>,
    chat_pins: Vec<ChatPinState>,
    private_contacts_v2: Vec<crate::private_contact_sync_v2::PrivateContactDocumentV2>,
    private_device_labels_v2: Vec<super::private_device_labels::PrivateDeviceLabel>,
    app_keys: Vec<DeviceSyncAppKeys>,
    groups: Vec<DeviceSyncGroup>,
    messages: Vec<DeviceSyncMessage>,
}
#[derive(Clone)]
enum DeviceSyncItem {
    Chat(DeviceSyncChat),
    Deletion(DeviceSyncChatDeletion),
    Mute(ChatMuteState),
    Pin(ChatPinState),
    PrivateContact(crate::private_contact_sync_v2::PrivateContactDocumentV2),
    PrivateDeviceLabel(super::private_device_labels::PrivateDeviceLabel),
    AppKeys(DeviceSyncAppKeys),
    Group(DeviceSyncGroup),
    Message(DeviceSyncMessage),
}
impl DeviceSyncItem {
    fn push(&self, snapshot: &mut DeviceSyncSnapshot) {
        match self {
            Self::Mute(value) => snapshot.chat_mutes.push(value.clone()),
            Self::Pin(value) => snapshot.chat_pins.push(value.clone()),
            Self::PrivateContact(value) => snapshot.private_contacts_v2.push(value.clone()),
            Self::PrivateDeviceLabel(value) => {
                snapshot.private_device_labels_v2.push(value.clone())
            }
            Self::Chat(value) => snapshot.chats.push(value.clone()),
            Self::Deletion(value) => snapshot.deleted_chats.push(value.clone()),
            Self::AppKeys(value) => snapshot.app_keys.push(value.clone()),
            Self::Group(value) => snapshot.groups.push(value.clone()),
            Self::Message(value) => snapshot.messages.push(value.clone()),
        }
    }

    fn pop(&self, snapshot: &mut DeviceSyncSnapshot) {
        match self {
            Self::PrivateDeviceLabel(_) => {
                snapshot.private_device_labels_v2.pop();
            }
            Self::PrivateContact(_) => {
                snapshot.private_contacts_v2.pop();
            }
            Self::Deletion(_) => {
                snapshot.deleted_chats.pop();
            }
            Self::Pin(_) => {
                snapshot.chat_pins.pop();
            }
            Self::Mute(_) => {
                snapshot.chat_mutes.pop();
            }
            Self::Chat(_) => {
                snapshot.chats.pop();
            }
            Self::AppKeys(_) => {
                snapshot.app_keys.pop();
            }
            Self::Group(_) => {
                snapshot.groups.pop();
            }
            Self::Message(_) => {
                snapshot.messages.pop();
            }
        }
    }
}

impl DeviceSyncSnapshot {
    fn packet(&self) -> DeviceSyncPacket {
        DeviceSyncPacket::Snapshot {
            v: DEVICE_SYNC_VERSION,
            roster_at: self.roster_at,
            chats: self.chats.clone(),
            deleted_chats: self.deleted_chats.clone(),
            chat_mutes: self.chat_mutes.clone(),
            chat_pins: self.chat_pins.clone(),
            private_contacts_v2: self.private_contacts_v2.clone(),
            private_device_labels_v2: self.private_device_labels_v2.clone(),
            app_keys: self.app_keys.clone(),
            groups: self.groups.clone(),
            messages: self.messages.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.chat_pins.is_empty()
            && self.private_contacts_v2.is_empty()
            && self.private_device_labels_v2.is_empty()
            && self.chat_mutes.is_empty()
            && self.deleted_chats.is_empty()
            && self.chats.is_empty()
            && self.app_keys.is_empty()
            && self.groups.is_empty()
            && self.messages.is_empty()
    }
}

impl AppCore {
    pub(super) fn handle_device_sync_packet(
        &mut self,
        source_pubkey_hex: &str,
        _source_port: u16,
        data: &[u8],
    ) {
        if data.len() > DEVICE_SYNC_MAX_PACKET_BYTES {
            return;
        }
        if !self.device_sync_peer_is_authorized(source_pubkey_hex) {
            self.clear_device_history(source_pubkey_hex);
            return;
        }
        let Ok(packet) = serde_json::from_slice::<DeviceSyncPacket>(data) else {
            return;
        };
        match packet {
            DeviceSyncPacket::Request {
                v,
                roster_at,
                page,
                history_reconcile,
                history_since,
                link_id,
            } if v == DEVICE_SYNC_VERSION => {
                self.negotiate_device_history(
                    source_pubkey_hex,
                    roster_at,
                    page.as_ref(),
                    history_reconcile,
                    history_since,
                );
                self.reply_device_sync_snapshot(
                    source_pubkey_hex,
                    roster_at,
                    page,
                    link_id.as_deref(),
                );
            }
            DeviceSyncPacket::ResyncRequired { v } if v == DEVICE_SYNC_VERSION => {
                self.clear_device_history(source_pubkey_hex);
                self.request_device_sync_snapshot(source_pubkey_hex, None);
            }
            DeviceSyncPacket::PageEnd {
                v,
                roster_at,
                next,
                history_reconcile,
                history_since,
            } if v == DEVICE_SYNC_VERSION => {
                if history_reconcile == Some(1)
                    && matches!(next, DeviceSyncPage::Messages { after: None })
                {
                    self.start_device_history(
                        source_pubkey_hex,
                        history_since.unwrap_or(roster_at),
                    );
                } else {
                    self.request_device_sync_snapshot(source_pubkey_hex, Some(next));
                }
            }
            DeviceSyncPacket::Snapshot {
                v,
                roster_at,
                chats,
                deleted_chats,
                chat_mutes,
                chat_pins,
                private_contacts_v2,
                private_device_labels_v2,
                app_keys,
                groups,
                messages,
            } if v == DEVICE_SYNC_VERSION => {
                self.apply_device_history_snapshot(
                    source_pubkey_hex,
                    DeviceSyncSnapshot {
                        roster_at,
                        chats,
                        deleted_chats,
                        chat_mutes,
                        chat_pins,
                        private_contacts_v2,
                        private_device_labels_v2,
                        app_keys,
                        groups,
                        messages,
                    },
                );
            }
            packet => self.handle_device_history(source_pubkey_hex, packet),
        }
    }

    pub(super) fn broadcast_device_sync_snapshot(&mut self) {
        let Some(runtime) = self.device_sync.as_mut() else {
            return;
        };
        let Some(tcp) = runtime.tcp.clone().filter(|_| !runtime.siblings.is_empty()) else {
            return;
        };
        if self.batch_depth > 0 {
            runtime.snapshot_pending = true;
            return;
        }
        let siblings = runtime.siblings.clone();
        let Some(roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let packets = metadata_page_packets(self, roster_at, 0);
        send_device_sync_packets(&tcp, &siblings, &packets);
    }

    pub(super) fn flush_device_sync_snapshot(&mut self) {
        if self
            .device_sync
            .as_mut()
            .is_some_and(|runtime| std::mem::take(&mut runtime.snapshot_pending))
        {
            self.broadcast_device_sync_snapshot();
        }
    }

    pub(super) fn broadcast_device_sync_message(&mut self, message: &ChatMessageSnapshot) {
        if !matches!(&message.kind, ChatMessageKind::User)
            || matches!(&message.delivery, DeliveryState::Failed)
            || (matches!(
                &message.delivery,
                DeliveryState::Queued | DeliveryState::Pending
            ) && !super::direct_files::is_pending_self_offer(
                &message.body,
                &message.chat_id,
                message.author_owner_pubkey_hex.as_deref(),
                message.is_outgoing,
            ))
        {
            return;
        }
        let Some(roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let Some(author) = message.author_owner_pubkey_hex.clone() else {
            return;
        };
        let packet = DeviceSyncSnapshot {
            roster_at,
            messages: vec![DeviceSyncMessage {
                chat_id: message.chat_id.clone(),
                id: message.id.clone(),
                body: message_wire_text(&message.body, &message.attachments),
                author,
                created_at: message.created_at_secs,
                expires_at: message.expires_at_secs,
            }],
            ..DeviceSyncSnapshot::default()
        }
        .packet();
        let Ok(packet) = serde_json::to_vec(&packet) else {
            return;
        };
        if packet.len() > DEVICE_SYNC_MAX_PACKET_BYTES {
            return;
        }
        let Some((tcp, siblings)) = self.device_sync.as_ref().and_then(|runtime| {
            runtime
                .tcp
                .clone()
                .map(|tcp| (tcp, runtime.siblings.clone()))
        }) else {
            return;
        };

        // Flush when possible before enqueueing. During a batched event this is
        // deferred; anti-entropy also reads the in-memory message projection.
        self.persist_best_effort();
        let recipients = siblings
            .into_iter()
            .filter(|peer| {
                self.device_history_send_since(&peer.pubkey().to_string())
                    .or_else(|| self.device_sync_peer_since(&peer.pubkey().to_string()))
                    .is_some_and(|since| message.created_at_secs >= since)
            })
            .collect::<Vec<_>>();
        send_device_sync_packets(&tcp, &recipients, std::slice::from_ref(&packet));
    }

    pub(super) fn device_sync_tracks_app_keys_owner(&self, owner: PublicKey) -> bool {
        let owner_hex = owner.to_hex();
        self.logged_in
            .as_ref()
            .is_some_and(|logged_in| logged_in.owner_pubkey == owner)
            || self.threads.contains_key(&owner_hex)
            || self.groups.values().any(|group| {
                group
                    .members
                    .iter()
                    .any(|member| member.to_hex() == owner_hex)
            })
    }

    fn build_device_sync_snapshot(
        &self,
        roster_at: u64,
        include_messages: bool,
    ) -> DeviceSyncSnapshot {
        let chat_ids = self
            .threads
            .keys()
            .chain(self.chat_read_states.keys())
            .filter(|id| valid_device_sync_chat_id(id))
            .cloned()
            .collect::<BTreeSet<_>>();
        let chats = chat_ids
            .into_iter()
            .filter_map(|id| {
                let read_state = self.chat_read_states.get(&id).cloned().filter(|state| {
                    self.chat_deletions
                        .get(&id)
                        .is_none_or(|deleted| state.updated_at_ms / 1000 > *deleted)
                });
                let thread = self.threads.get(&id);
                if thread.is_none() && read_state.is_none() {
                    return None;
                }
                Some(DeviceSyncChat {
                    updated_at: thread.map_or_else(
                        || {
                            read_state
                                .as_ref()
                                .map_or(0, |state| state.seen_through_secs)
                        },
                        |thread| thread.updated_at_secs,
                    ),
                    // Legacy receivers would bridge these fields to static-key records.
                    id,
                    read_state,
                })
            })
            .collect::<Vec<_>>();
        let direct_chat_ids = chats
            .iter()
            .filter_map(|chat| PublicKey::from_hex(&chat.id).ok())
            .map(|owner| owner.to_hex())
            .collect::<BTreeSet<_>>();
        let mut app_key_owners = direct_chat_ids.clone();
        app_key_owners.extend(
            self.app_keys
                .values()
                .filter(|known| {
                    known
                        .devices
                        .iter()
                        .any(|device| direct_chat_ids.contains(&device.identity_pubkey_hex))
                })
                .map(|known| known.owner_pubkey_hex.clone()),
        );
        if let Some(logged_in) = self.logged_in.as_ref() {
            app_key_owners.insert(logged_in.owner_pubkey.to_hex());
        }
        app_key_owners.extend(
            self.groups
                .values()
                .flat_map(|group| group.members.iter().map(|member| member.to_hex())),
        );
        let app_keys = app_key_owners
            .into_iter()
            .filter_map(|owner| {
                self.app_keys
                    .get(&owner)
                    .and_then(|known| DeviceSyncAppKeys::from_known(&owner, known))
            })
            .collect();
        let groups = self
            .groups
            .values()
            .map(|group| DeviceSyncGroup {
                id: group.group_id.clone(),
                name: group.name.clone(),
                description: group.about.clone(),
                picture: group.picture.clone(),
                created_by: group.created_by.to_hex(),
                members: group.members.iter().map(|member| member.to_hex()).collect(),
                admins: group.admins.iter().map(|admin| admin.to_hex()).collect(),
                protocol: Some(
                    match group.protocol.strategy {
                        GroupStrategy::PairwiseFanout => "pairwise_fanout_v1",
                        GroupStrategy::SenderKey => "sender_key_v1",
                    }
                    .to_string(),
                ),
                revision: group.revision,
                created_at: group.created_at.get(),
                updated_at: group.updated_at.get(),
                accepted: Some(true),
            })
            .collect();
        let messages = if include_messages {
            collect_device_sync_messages(self, roster_at, None, DEVICE_SYNC_PAGE_MESSAGES).0
        } else {
            Vec::new()
        };
        DeviceSyncSnapshot {
            roster_at,
            chats,
            chat_mutes: self.chat_mute_snapshot(),
            chat_pins: self.chat_pin_snapshot(),
            private_contacts_v2: self.private_contact_snapshot(),
            private_device_labels_v2: self.private_device_label_snapshot(),
            deleted_chats: self
                .chat_deletions
                .iter()
                .map(|(id, deleted_at)| DeviceSyncChatDeletion {
                    id: id.clone(),
                    deleted_at: *deleted_at,
                })
                .collect(),
            app_keys,
            groups,
            messages,
        }
    }

    fn apply_device_sync_snapshot(
        &mut self,
        snapshot: DeviceSyncSnapshot,
        history_since: Option<u64>,
    ) {
        let Some(local_roster_at) = self.device_sync_roster_at() else {
            return;
        };

        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return;
        };
        let mut changed = false;
        for state in snapshot.chat_pins {
            match self.merge_chat_pin(state) {
                Ok(merged) => changed |= merged,
                Err(error) => self.push_debug_log("chat_pin.save_failed", error.to_string()),
            }
        }
        for state in snapshot.chat_mutes {
            match self.merge_chat_mute(state) {
                Ok(applied) => changed |= applied,
                Err(error) => self.push_debug_log("chat_mute.save_failed", error.to_string()),
            }
        }
        for deletion in snapshot.deleted_chats {
            if valid_device_sync_chat_id(&deletion.id)
                && deletion.deleted_at > 0
                && deletion.deleted_at <= unix_now().get().saturating_add(300)
            {
                changed |= self.apply_chat_deletion(&deletion.id, deletion.deleted_at);
            }
        }
        let mut app_keys_changed = false;
        let mut app_keys_retry_batch = ProtocolRetryBatch::default();

        for app_keys in snapshot.app_keys {
            let Some((owner, mut incoming, created_at)) = app_keys.into_app_keys() else {
                continue;
            };
            let owner_hex = owner.to_hex();
            let current = self.app_keys.get(&owner_hex).cloned();
            preserve_known_app_key_labels(current.as_ref(), &mut incoming);
            let (_, mut known) = canonical_known_app_keys_snapshot(
                current.as_ref(),
                owner,
                &incoming,
                created_at,
                None,
            );
            // Names have their own clock. A stale membership snapshot can still
            // carry a newer name for a device that remains authorized.
            let label_source = known_app_keys_from_ndr(owner, &incoming, created_at);
            account_app_keys::merge_known_device_labels(&mut known, &label_source);
            let effective = known_app_keys_to_ndr(&known);
            if current.as_ref() == Some(&known) {
                continue;
            }
            if let Err(error) =
                self.invalidate_removed_device_history(owner, current.as_ref(), &known)
            {
                self.push_debug_log("device_sync.history_revoke.error", error.to_string());
                continue;
            }
            let retry_batch = match self.protocol_engine.as_mut() {
                Some(engine) => match engine.ingest_app_keys_snapshot(
                    owner,
                    effective.clone(),
                    known.created_at_secs,
                ) {
                    Ok(batch) => batch,
                    Err(error) => {
                        self.push_debug_log("device_sync.app_keys.error", error.to_string());
                        continue;
                    }
                },
                None => ProtocolRetryBatch::default(),
            };
            self.app_keys.insert(owner_hex, known);
            self.migrate_verified_device_owner_threads(owner, &effective);
            Self::append_protocol_retry_batch(&mut app_keys_retry_batch, retry_batch);
            app_keys_changed = true;
        }
        if app_keys_changed {
            self.reconcile_device_sync();
            self.mark_mobile_push_dirty();
            self.refresh_local_authorization_state();
            changed = true;
        }

        let cutoff = history_since.unwrap_or(local_roster_at.max(snapshot.roster_at));

        for chat in &snapshot.chats {
            if PublicKey::from_hex(&chat.id).is_ok()
                && !self.threads.contains_key(&chat.id)
                && !self.chat_activity_is_deleted(&chat.id, chat.updated_at)
            {
                self.ensure_thread_record(&chat.id, chat.updated_at);
                changed = true;
            }
        }
        for group in snapshot.groups {
            if self.chat_activity_is_deleted(&group_chat_id(&group.id), group.updated_at) {
                continue;
            }
            let Some(group) = group.into_group_snapshot() else {
                continue;
            };
            let installed = self
                .protocol_engine
                .as_mut()
                .and_then(|engine| engine.install_device_sync_group(group.clone()).ok())
                .unwrap_or(false);
            if installed {
                let previous = self.groups.get(&group.group_id).cloned();
                if self.apply_group_roster_snapshot(group.clone(), group.updated_at.get()) {
                    self.apply_group_metadata_notice(previous.as_ref(), &group);
                    changed = true;
                }
            }
        }
        for chat in snapshot.chats {
            if valid_device_sync_chat_id(&chat.id) {
                if let Some(read_state) = chat.read_state {
                    changed |= self.apply_chat_read_state(&chat.id, read_state);
                }
            }
        }
        for label in snapshot.private_device_labels_v2 {
            changed |= self.merge_private_device_label(label);
        }
        for document in snapshot.private_contacts_v2 {
            changed |= self.merge_private_contact_from_sibling(&document);
        }
        let now = unix_now().get();
        for message in snapshot.messages {
            if self.chat_activity_is_deleted(&message.chat_id, message.created_at)
                || message.created_at < cutoff
                || message
                    .expires_at
                    .is_some_and(|expires_at| expires_at <= now)
                || !valid_device_sync_chat_id(&message.chat_id)
                || message.id.is_empty()
                || message.id.len() > 128
                || message.body.len() > 32 * 1024
                || PublicKey::from_hex(&message.author).is_err()
                || self.threads.get(&message.chat_id).is_some_and(|thread| {
                    thread.messages.iter().any(|known| known.id == message.id)
                })
                || self
                    .app_store
                    .message_exists_or_deleted(&message.chat_id, Some(&message.id), None)
                    .unwrap_or(true)
            {
                continue;
            }
            let is_outgoing = message.author == local_owner_hex;
            let chat_id = message.chat_id.clone();
            let (body, attachments) = extract_message_attachments(&message.body);
            let already_seen = !is_outgoing
                && self.message_was_seen_on_own_device(&chat_id, message.created_at, &message.id);
            let count_unread = !is_outgoing && !already_seen && !self.is_chat_visible(&chat_id);
            if is_outgoing {
                self.accept_direct_peer(&chat_id);
            }
            let thread = self.ensure_thread_record(&chat_id, message.created_at);
            thread.updated_at_secs = thread.updated_at_secs.max(message.created_at);
            if count_unread {
                thread.unread_count = thread.unread_count.saturating_add(1);
            }
            thread.insert_message_sorted(ChatMessageSnapshot {
                system_notice_owner_pubkey_hex: None,
                direct_transfer: None,
                call: None,
                id: message.id,
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: message.author.clone(),
                author_owner_pubkey_hex: Some(message.author),
                author_picture_url: None,
                body,
                attachments,
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing,
                created_at_secs: message.created_at,
                expires_at_secs: message.expires_at,
                delivery: if is_outgoing {
                    DeliveryState::Sent
                } else if already_seen {
                    DeliveryState::Seen
                } else {
                    DeliveryState::Received
                },
                recipient_deliveries: Vec::new(),
                delivery_trace: MessageDeliveryTraceSnapshot::default(),
                source_event_id: None,
            });
            self.bump_typing_floor(&chat_id, message.created_at);
            if message.expires_at.is_some() {
                self.schedule_next_message_expiry();
            }
            changed = true;
        }
        if changed {
            self.request_protocol_subscription_refresh();
            self.persist_best_effort();
            self.rebuild_state();
            self.emit_state();
        }
        if !app_keys_retry_batch.is_empty() {
            self.process_protocol_engine_retry_batch("device_sync_app_keys", app_keys_retry_batch);
        }
    }

    fn device_sync_roster_at(&self) -> Option<u64> {
        let logged_in = self.logged_in.as_ref()?;
        let roster = self.app_keys.get(&logged_in.owner_pubkey.to_hex())?;
        // Membership updates must not move an existing device's history window.
        roster
            .devices
            .iter()
            .find(|device| {
                device.identity_pubkey_hex == logged_in.device_keys.public_key().to_hex()
            })
            .map(|device| device.created_at_secs)
            .filter(|created_at| *created_at > 0)
    }

    fn device_sync_peer_since(&self, source_pubkey_hex: &str) -> Option<u64> {
        let logged_in = self.logged_in.as_ref()?;
        self.app_keys
            .get(&logged_in.owner_pubkey.to_hex())?
            .devices
            .iter()
            .find(|device| {
                device
                    .identity_pubkey_hex
                    .eq_ignore_ascii_case(source_pubkey_hex)
            })
            .map(|device| device.created_at_secs)
            .filter(|created_at| *created_at > 0)
    }

    pub(super) fn device_sync_peer_is_authorized(&self, source_pubkey_hex: &str) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        source_pubkey_hex != logged_in.device_keys.public_key().to_hex()
            && self
                .app_keys
                .get(&logged_in.owner_pubkey.to_hex())
                .is_some_and(|roster| {
                    roster.devices.iter().any(|device| {
                        device.identity_pubkey_hex == logged_in.device_keys.public_key().to_hex()
                    }) && roster.devices.iter().any(|device| {
                        device
                            .identity_pubkey_hex
                            .eq_ignore_ascii_case(source_pubkey_hex)
                    })
                })
    }
}

impl DeviceSyncGroup {
    fn into_group_snapshot(self) -> Option<GroupSnapshot> {
        if self.id.is_empty() || self.id.len() > 128 || self.name.len() > 4096 {
            return None;
        }
        let created_by = ndr_owner_from_hex(&self.created_by)?;
        let members = self
            .members
            .iter()
            .map(|value| ndr_owner_from_hex(value))
            .collect::<Option<Vec<_>>>()?;
        let admins = self
            .admins
            .iter()
            .map(|value| ndr_owner_from_hex(value))
            .collect::<Option<Vec<_>>>()?;
        if members.is_empty() {
            return None;
        }
        Some(GroupSnapshot {
            group_id: self.id,
            protocol: match self.protocol.as_deref() {
                Some("sender_key_v1") => GroupProtocol::sender_key_v1(),
                _ => GroupProtocol::pairwise_fanout_v1(),
            },
            name: self.name,
            picture: self.picture,
            about: self.description,
            created_by,
            members,
            admins,
            revision: self.revision,
            created_at: NdrUnixSeconds(self.created_at),
            updated_at: NdrUnixSeconds(self.updated_at),
        })
    }
}

impl DeviceSyncAppKeys {
    fn from_known(owner_pubkey: &str, known: &KnownAppKeys) -> Option<Self> {
        let owner_pubkey = PublicKey::from_hex(owner_pubkey).ok()?.to_hex();
        if !known.owner_pubkey_hex.eq_ignore_ascii_case(&owner_pubkey) {
            return None;
        }
        let mut devices = known
            .devices
            .iter()
            .map(|device| {
                Some(DeviceSyncAppKeyDevice {
                    identity_pubkey: PublicKey::from_hex(&device.identity_pubkey_hex)
                        .ok()?
                        .to_hex(),
                    created_at: device.created_at_secs,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        devices.sort_by(|left, right| left.identity_pubkey.cmp(&right.identity_pubkey));
        devices.dedup_by(|left, right| left.identity_pubkey == right.identity_pubkey);
        Some(Self {
            owner_pubkey,
            created_at: known.created_at_secs,
            devices,
        })
    }

    fn into_app_keys(self) -> Option<(PublicKey, AppKeys, u64)> {
        let owner = PublicKey::from_hex(&self.owner_pubkey).ok()?;
        let mut identities = HashSet::new();
        let mut incoming = AppKeys::new(Vec::new());
        for device in self.devices {
            let identity = PublicKey::from_hex(&device.identity_pubkey).ok()?;
            if !identities.insert(identity) {
                return None;
            }
            incoming.add_device(DeviceEntry::new(identity, device.created_at));
        }
        Some((owner, incoming, self.created_at))
    }
}

fn encode_device_sync_chunks(snapshot: DeviceSyncSnapshot) -> Vec<Vec<u8>> {
    let roster_at = snapshot.roster_at;
    let items = snapshot
        .deleted_chats
        .into_iter()
        .map(DeviceSyncItem::Deletion)
        .chain(snapshot.chat_mutes.into_iter().map(DeviceSyncItem::Mute))
        .chain(snapshot.chat_pins.into_iter().map(DeviceSyncItem::Pin))
        .chain(
            snapshot
                .private_contacts_v2
                .into_iter()
                .map(DeviceSyncItem::PrivateContact),
        )
        .chain(
            snapshot
                .private_device_labels_v2
                .into_iter()
                .map(DeviceSyncItem::PrivateDeviceLabel),
        )
        .chain(snapshot.chats.into_iter().map(DeviceSyncItem::Chat))
        .chain(snapshot.app_keys.into_iter().map(DeviceSyncItem::AppKeys))
        .chain(snapshot.groups.into_iter().map(DeviceSyncItem::Group))
        .chain(snapshot.messages.into_iter().map(DeviceSyncItem::Message));
    let mut current = DeviceSyncSnapshot {
        roster_at,
        ..DeviceSyncSnapshot::default()
    };
    let mut packets = Vec::new();
    for item in items {
        item.push(&mut current);
        if serde_json::to_vec(&current.packet())
            .is_ok_and(|data| data.len() <= DEVICE_SYNC_MAX_PACKET_BYTES)
        {
            continue;
        }
        item.pop(&mut current);
        if !current.is_empty() {
            let Ok(packet) = serde_json::to_vec(&current.packet()) else {
                return Vec::new();
            };
            packets.push(packet);
        }
        current = DeviceSyncSnapshot {
            roster_at,
            ..DeviceSyncSnapshot::default()
        };
        item.push(&mut current);
        if serde_json::to_vec(&current.packet())
            .is_ok_and(|data| data.len() > DEVICE_SYNC_MAX_PACKET_BYTES)
        {
            item.pop(&mut current);
        }
    }
    if !current.is_empty() || packets.is_empty() {
        let Ok(packet) = serde_json::to_vec(&current.packet()) else {
            return Vec::new();
        };
        packets.push(packet);
    }
    packets
}

pub(super) fn fips_peer_from_hex(pubkey_hex: &str) -> Option<FipsPeerIdentity> {
    let pubkey = PublicKey::from_hex(pubkey_hex).ok()?;
    FipsPeerIdentity::from_npub(&pubkey.to_bech32().ok()?).ok()
}

fn send_device_sync_packets(
    tcp: &DeviceSyncTcpSender,
    siblings: &[FipsPeerIdentity],
    packets: &[Vec<u8>],
) {
    for sibling in siblings {
        let _ = tcp.send_batch(*sibling, packets.to_vec());
    }
}

fn ndr_owner_from_hex(pubkey_hex: &str) -> Option<NdrOwnerPubkey> {
    PublicKey::from_hex(pubkey_hex)
        .ok()
        .map(|pubkey| NdrOwnerPubkey::from_bytes(pubkey.to_bytes()))
}

#[cfg(test)]
#[path = "device_sync_tests.rs"]
mod tests;
