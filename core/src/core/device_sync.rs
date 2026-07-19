use super::*;
use fips_core::{FipsEndpoint, PeerIdentity as FipsPeerIdentity};
use nostr_double_ratchet::{GroupProtocol, GroupStrategy};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};
use nostr_pubsub_relay::RelayEventBus;
use std::collections::BTreeSet;
use tokio::task::JoinHandle;

use anti_entropy::metadata_page_packets;
use messages::collect_device_sync_messages;

mod anti_entropy;
mod body;
mod messages;
mod runtime;

pub(super) const DEVICE_SYNC_PORT: u16 = 7369;
const DEVICE_SYNC_VERSION: u8 = 1;
const DEVICE_SYNC_MAX_PACKET_BYTES: usize = 64 * 1024;
const DEVICE_SYNC_PAGE_MESSAGES: usize = 32;
const DEVICE_SYNC_PAGE_PACKETS: usize = 32;
const DEVICE_SYNC_SCOPE_PREFIX: &str = "iris-chat-device-sync-v1:";
struct DeviceSyncConfig {
    key: String,
    owner_hex: String,
    roster_at: u64,
    secret_hex: String,
    relay_urls: Vec<String>,
    relay_client: Option<Client>,
    siblings: Vec<FipsPeerIdentity>,
}
pub(super) struct DeviceSyncRuntime {
    key: String,
    endpoint: Arc<FipsEndpoint>,
    tcp: Option<DeviceSyncTcpSender>,
    siblings: Vec<FipsPeerIdentity>,
    _attachment_blobs: Option<Arc<super::attachment_upload::AttachmentBlobRuntime>>,
    _update_pubsub: Option<Arc<FipsPubsubClient>>,
    _update_relay_pubsub: Option<Arc<RelayEventBus>>,
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
    },
    ResyncRequired {
        v: u8,
    },
    PageEnd {
        v: u8,
        roster_at: u64,
        next: DeviceSyncPage,
    },
    Snapshot {
        v: u8,
        roster_at: u64,
        #[serde(default)]
        chats: Vec<DeviceSyncChat>,
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
    app_keys: Vec<DeviceSyncAppKeys>,
    groups: Vec<DeviceSyncGroup>,
    messages: Vec<DeviceSyncMessage>,
}
#[derive(Clone)]
enum DeviceSyncItem {
    Chat(DeviceSyncChat),
    AppKeys(DeviceSyncAppKeys),
    Group(DeviceSyncGroup),
    Message(DeviceSyncMessage),
}
impl DeviceSyncItem {
    fn push(&self, snapshot: &mut DeviceSyncSnapshot) {
        match self {
            Self::Chat(value) => snapshot.chats.push(value.clone()),
            Self::AppKeys(value) => snapshot.app_keys.push(value.clone()),
            Self::Group(value) => snapshot.groups.push(value.clone()),
            Self::Message(value) => snapshot.messages.push(value.clone()),
        }
    }

    fn pop(&self, snapshot: &mut DeviceSyncSnapshot) {
        match self {
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
            app_keys: self.app_keys.clone(),
            groups: self.groups.clone(),
            messages: self.messages.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.chats.is_empty()
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
        if data.len() > DEVICE_SYNC_MAX_PACKET_BYTES
            || !self.device_sync_peer_is_authorized(source_pubkey_hex)
        {
            return;
        }
        let Ok(packet) = serde_json::from_slice::<DeviceSyncPacket>(data) else {
            return;
        };
        match packet {
            DeviceSyncPacket::Request { v, roster_at, page } if v == DEVICE_SYNC_VERSION => {
                self.reply_device_sync_snapshot(source_pubkey_hex, roster_at, page);
            }
            DeviceSyncPacket::ResyncRequired { v } if v == DEVICE_SYNC_VERSION => {
                self.request_device_sync_snapshot(source_pubkey_hex, None);
            }
            DeviceSyncPacket::PageEnd {
                v,
                roster_at: _,
                next,
            } if v == DEVICE_SYNC_VERSION => {
                self.request_device_sync_snapshot(source_pubkey_hex, Some(next));
            }
            DeviceSyncPacket::Snapshot {
                v,
                roster_at,
                chats,
                app_keys,
                groups,
                messages,
            } if v == DEVICE_SYNC_VERSION => {
                self.apply_device_sync_snapshot(DeviceSyncSnapshot {
                    roster_at,
                    chats,
                    app_keys,
                    groups,
                    messages,
                });
            }
            _ => {}
        }
    }

    pub(super) fn broadcast_device_sync_snapshot(&mut self) {
        let Some(roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let packets = metadata_page_packets(self, roster_at, 0);
        let Some((tcp, siblings)) = self.device_sync.as_ref().and_then(|runtime| {
            runtime
                .tcp
                .clone()
                .map(|tcp| (tcp, runtime.siblings.clone()))
        }) else {
            return;
        };
        send_device_sync_packets(&tcp, &siblings, &packets);
    }

    pub(super) fn broadcast_device_sync_message(&mut self, message: &ChatMessageSnapshot) {
        if !matches!(&message.kind, ChatMessageKind::User)
            || matches!(
                &message.delivery,
                DeliveryState::Queued | DeliveryState::Pending | DeliveryState::Failed
            )
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
                body: message.body.clone(),
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
        send_device_sync_packets(&tcp, &siblings, std::slice::from_ref(&packet));
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
        let chats = self
            .threads
            .values()
            .filter(|thread| PublicKey::from_hex(&thread.chat_id).is_ok())
            .map(|thread| DeviceSyncChat {
                id: thread.chat_id.clone(),
                updated_at: thread.updated_at_secs,
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
            app_keys,
            groups,
            messages,
        }
    }

    fn apply_device_sync_snapshot(&mut self, snapshot: DeviceSyncSnapshot) {
        let Some(local_roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let cutoff = local_roster_at.max(snapshot.roster_at);
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_hex())
        else {
            return;
        };
        let mut changed = false;
        let mut app_keys_changed = false;
        let mut app_keys_retry_batch = ProtocolRetryBatch::default();

        for app_keys in snapshot.app_keys {
            let Some((owner, mut incoming, created_at)) = app_keys.into_app_keys() else {
                continue;
            };
            let owner_hex = owner.to_hex();
            let current = self.app_keys.get(&owner_hex).cloned();
            preserve_known_app_key_labels(current.as_ref(), &mut incoming);
            let (effective, known) = canonical_known_app_keys_snapshot(
                current.as_ref(),
                owner,
                &incoming,
                created_at,
                None,
            );
            if current.as_ref() == Some(&known) {
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

        for chat in snapshot.chats {
            if PublicKey::from_hex(&chat.id).is_ok() && !self.threads.contains_key(&chat.id) {
                self.ensure_thread_record(&chat.id, chat.updated_at);
                changed = true;
            }
        }
        for group in snapshot.groups {
            let Some(group) = group.into_group_snapshot(&local_owner_hex) else {
                continue;
            };
            let installed = self
                .protocol_engine
                .as_mut()
                .and_then(|engine| engine.install_device_sync_group(group.clone()).ok())
                .unwrap_or(false);
            if installed {
                self.apply_group_roster_snapshot(group.clone(), group.updated_at.get());
                changed = true;
            }
        }
        let now = unix_now().get();
        for message in snapshot.messages {
            if message.created_at < cutoff
                || message
                    .expires_at
                    .is_some_and(|expires_at| expires_at <= now)
                || !valid_device_sync_chat_id(&message.chat_id)
                || message.id.is_empty()
                || message.id.len() > 128
                || PublicKey::from_hex(&message.author).is_err()
                || self.threads.get(&message.chat_id).is_some_and(|thread| {
                    thread.messages.iter().any(|known| known.id == message.id)
                })
                || self
                    .app_store
                    .message_exists(&message.chat_id, Some(&message.id), None)
                    .unwrap_or(true)
            {
                continue;
            }
            let is_outgoing = message.author == local_owner_hex;
            let chat_id = message.chat_id.clone();
            if is_outgoing {
                self.accept_direct_peer(&chat_id);
            }
            self.ensure_thread_record(&chat_id, message.created_at)
                .insert_message_sorted(ChatMessageSnapshot {
                    id: message.id,
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: message.author.clone(),
                    author_owner_pubkey_hex: Some(message.author),
                    author_picture_url: None,
                    body: message.body,
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing,
                    created_at_secs: message.created_at,
                    expires_at_secs: message.expires_at,
                    delivery: if is_outgoing {
                        DeliveryState::Sent
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

    #[cfg(test)]
    pub(super) fn build_device_sync_packets_for_test(
        &self,
        roster_at: u64,
        include_messages: bool,
    ) -> Vec<Vec<u8>> {
        encode_device_sync_chunks(self.build_device_sync_snapshot(roster_at, include_messages))
    }

    #[cfg(test)]
    pub(super) fn install_device_sync_sender_for_test(
        &mut self,
        endpoint: Arc<FipsEndpoint>,
        tcp: DeviceSyncTcpSender,
        siblings: Vec<FipsPeerIdentity>,
    ) {
        self.device_sync = Some(DeviceSyncRuntime {
            key: "test".to_string(),
            endpoint,
            tcp: Some(tcp),
            siblings,
            _attachment_blobs: None,
            _update_pubsub: None,
            _update_relay_pubsub: None,
            tasks: Vec::new(),
        });
    }

    #[cfg(test)]
    pub(super) fn take_device_sync_control_for_test(
        &self,
        peer: FipsPeerIdentity,
    ) -> Option<Vec<u8>> {
        self.device_sync
            .as_ref()
            .and_then(|runtime| runtime.tcp.as_ref())
            .and_then(|tcp| tcp.take_control_for_test(peer))
    }

    fn device_sync_roster_at(&self) -> Option<u64> {
        let logged_in = self.logged_in.as_ref()?;
        let roster = self.app_keys.get(&logged_in.owner_pubkey.to_hex())?;
        roster
            .devices
            .iter()
            .any(|device| device.identity_pubkey_hex == logged_in.device_keys.public_key().to_hex())
            .then_some(roster.created_at_secs)
            .filter(|created_at| *created_at > 0)
    }

    fn device_sync_peer_is_authorized(&self, source_pubkey_hex: &str) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        source_pubkey_hex != logged_in.device_keys.public_key().to_hex()
            && self
                .app_keys
                .get(&logged_in.owner_pubkey.to_hex())
                .is_some_and(|roster| {
                    roster.devices.iter().any(|device| {
                        device
                            .identity_pubkey_hex
                            .eq_ignore_ascii_case(source_pubkey_hex)
                    })
                })
    }
}

impl DeviceSyncGroup {
    fn into_group_snapshot(self, local_owner_hex: &str) -> Option<GroupSnapshot> {
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
        if !self
            .members
            .iter()
            .any(|member| member.eq_ignore_ascii_case(local_owner_hex))
            || members.is_empty()
        {
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
        let devices = self
            .devices
            .into_iter()
            .map(|device| {
                let identity = PublicKey::from_hex(&device.identity_pubkey).ok()?;
                identities
                    .insert(identity)
                    .then_some(DeviceEntry::new(identity, device.created_at))
            })
            .collect::<Option<Vec<_>>>()?;
        Some((owner, AppKeys::new(devices), self.created_at))
    }
}

fn encode_device_sync_chunks(snapshot: DeviceSyncSnapshot) -> Vec<Vec<u8>> {
    let roster_at = snapshot.roster_at;
    let items = snapshot
        .chats
        .into_iter()
        .map(DeviceSyncItem::Chat)
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

fn fips_peer_from_hex(pubkey_hex: &str) -> Option<FipsPeerIdentity> {
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

fn valid_device_sync_chat_id(chat_id: &str) -> bool {
    chat_id
        .strip_prefix("group:")
        .is_some_and(|group_id| !group_id.is_empty() && group_id.len() <= 128)
        || PublicKey::from_hex(chat_id).is_ok()
}

#[cfg(test)]
#[path = "device_sync_tests.rs"]
mod tests;
