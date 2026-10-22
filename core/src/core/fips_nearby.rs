use super::*;
use crate::{DesktopNearbyPeerSnapshot, DesktopNearbySnapshot};
use fips_core::transport::ble::host::HostBleIo;
use fips_core::PeerIdentity as FipsPeerIdentity;
use std::collections::BTreeSet;
pub(in crate::core) mod diagnostics;
mod private_targets;
pub(super) use diagnostics::ReceiveDiagnostics;
pub(in crate::core) use private_targets::is_private_transport_event;

pub(super) const FIPS_NEARBY_PORT: u16 = 7370;
pub(super) const FIPS_NEARBY_SCOPE: &str = "iris-chat-nearby-v1";
const FIPS_NEARBY_VERSION: u8 = 1;
const FIPS_NEARBY_MAX_PACKET_BYTES: usize = 48 * 1024;
pub(super) const FIPS_NEARBY_OUTBOX_MAX_EVENTS: usize = 256;
const FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct HostBleOwnership(Arc<std::sync::atomic::AtomicBool>);

impl HostBleOwnership {
    pub(crate) fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub(crate) struct HostBleAttachment {
    io: Option<HostBleIo>,
    ownership: HostBleOwnership,
    background_capable: bool,
}

impl HostBleAttachment {
    pub(crate) fn new(io: HostBleIo) -> Self {
        Self {
            io: Some(io),
            ownership: HostBleOwnership(Arc::new(std::sync::atomic::AtomicBool::new(false))),
            background_capable: false,
        }
    }

    pub(crate) fn with_background(mut self, enabled: bool) -> Self {
        self.background_capable = enabled;
        self
    }

    pub(crate) fn ownership(&self) -> HostBleOwnership {
        self.ownership.clone()
    }

    pub(super) fn take(&mut self) -> Option<HostBleIo> {
        let io = self.io.take();
        if self.ownership.is_cancelled() {
            None
        } else {
            io
        }
    }
}

impl std::fmt::Debug for HostBleAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostBleAttachment")
            .field("available", &self.io.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(super) enum FipsNearbyPacket {
    Event {
        v: u8,
        event_id: String,
        event_json: String,
    },
    Receipt {
        v: u8,
        event_id: String,
    },
}

impl FipsNearbyPacket {
    pub(super) fn event(event_id: String, event_json: String) -> Option<Self> {
        (valid_event_id(&event_id) && event_json.len() <= FIPS_NEARBY_MAX_PACKET_BYTES).then_some(
            Self::Event {
                v: FIPS_NEARBY_VERSION,
                event_id,
                event_json,
            },
        )
    }

    pub(super) fn receipt(event_id: String) -> Option<Self> {
        valid_event_id(&event_id).then_some(Self::Receipt {
            v: FIPS_NEARBY_VERSION,
            event_id,
        })
    }

    pub(super) fn encode(&self) -> Option<Vec<u8>> {
        let encoded = serde_json::to_vec(self).ok()?;
        (encoded.len() <= FIPS_NEARBY_MAX_PACKET_BYTES).then_some(encoded)
    }

    #[cfg(test)]
    pub(super) fn decode(data: &[u8]) -> Option<Self> {
        Self::decode_diagnostic(data).ok()
    }

    fn decode_diagnostic(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() > FIPS_NEARBY_MAX_PACKET_BYTES {
            return Err("packet_size");
        }
        let packet = serde_json::from_slice::<Self>(data).map_err(|_| "packet_json")?;
        let (version, event_id) = match &packet {
            Self::Event { v, event_id, .. } | Self::Receipt { v, event_id } => (*v, event_id),
        };
        if version != FIPS_NEARBY_VERSION {
            return Err("packet_version");
        }
        if !valid_event_id(event_id) {
            return Err("packet_id");
        }
        match &packet {
            Self::Event { event_json, .. } if event_json.len() > FIPS_NEARBY_MAX_PACKET_BYTES => {
                Err("event_size")
            }
            _ => Ok(packet),
        }
    }
}

fn valid_event_id(event_id: &str) -> bool {
    event_id.len() == 64 && event_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn encode_fips_nearby_event(event: &Event) -> Option<Vec<u8>> {
    FipsNearbyPacket::event(event.id.to_string(), serde_json::to_string(event).ok()?)?.encode()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FipsNearbyOutboxLink {
    peer_npub: String,
    link_id: u64,
    attempts: u8,
    retry_at: Instant,
}

#[derive(Clone, Debug)]
struct FipsNearbyOutboxEntry {
    event_id: String,
    payload: Vec<u8>,
    sent_links: VecDeque<FipsNearbyOutboxLink>,
    acknowledged_peers: VecDeque<String>,
    // None is public bootstrap/gossip. Private envelopes have exact live
    // recipients, including an empty set while an invite mapping is unavailable.
    recipient_targets: Option<BTreeSet<String>>,
}

impl FipsNearbyOutboxEntry {
    fn acknowledged_by(&self, peer_npub: &str) -> bool {
        self.acknowledged_peers.iter().any(|peer| peer == peer_npub)
    }
}

/// Bounded nearby-event queue shared with the FIPS link monitor.
///
/// Transport acceptance is not a delivery acknowledgement. Retry unacknowledged
/// events on the same link with exponential backoff (2 to 30 seconds), or
/// immediately on a new link. A receipt stops retries to that peer only: another
/// peer may still need the event when it connects. Retention remains bounded.
#[derive(Default, Debug)]
pub(super) struct FipsNearbyOutbox {
    entries: VecDeque<FipsNearbyOutboxEntry>,
}

impl FipsNearbyOutbox {
    pub(super) fn has_pending_private_for_peer(&self, peer_npub: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry
                .recipient_targets
                .as_ref()
                .is_some_and(|targets| targets.contains(peer_npub))
                && !entry.acknowledged_by(peer_npub)
        })
    }

    #[cfg(test)]
    pub(super) fn insert(&mut self, event_id: String, payload: Vec<u8>) {
        let targets = private_targets::encoded_private_targets(&payload);
        self.insert_for_targets(event_id, payload, targets);
    }

    pub(super) fn insert_for_targets(
        &mut self,
        event_id: String,
        payload: Vec<u8>,
        recipient_targets: Option<BTreeSet<String>>,
    ) {
        let (sent_links, acknowledged_peers) = self
            .entries
            .iter()
            .position(|entry| entry.event_id == event_id)
            .and_then(|index| self.entries.remove(index))
            .map(|entry| (entry.sent_links, entry.acknowledged_peers))
            .unwrap_or_default();
        while self.entries.len() >= FIPS_NEARBY_OUTBOX_MAX_EVENTS {
            self.entries.pop_front();
        }
        self.entries.push_back(FipsNearbyOutboxEntry {
            event_id,
            payload,
            sent_links,
            acknowledged_peers,
            recipient_targets,
        });
    }

    #[cfg(test)]
    pub(super) fn pending_for_link(&self, peer_npub: &str, link_id: u64) -> Vec<(String, Vec<u8>)> {
        self.pending_for_link_with_public(peer_npub, link_id, true)
    }

    pub(super) fn pending_for_link_with_public(
        &self,
        peer_npub: &str,
        link_id: u64,
        allow_public: bool,
    ) -> Vec<(String, Vec<u8>)> {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|entry| allow_public || entry.recipient_targets.is_some())
            .filter(|entry| {
                entry
                    .recipient_targets
                    .as_ref()
                    .is_none_or(|targets| targets.contains(peer_npub))
                    && !entry.acknowledged_by(peer_npub)
                    && !entry.sent_links.iter().any(|link| {
                        link.peer_npub == peer_npub
                            && link.link_id == link_id
                            && now < link.retry_at
                    })
            })
            .map(|entry| (entry.event_id.clone(), entry.payload.clone()))
            .collect()
    }

    pub(super) fn mark_sent_on_link(
        &mut self,
        peer_npub: &str,
        link_id: u64,
        event_ids: &[String],
    ) {
        let now = Instant::now();
        for entry in self
            .entries
            .iter_mut()
            .filter(|entry| event_ids.contains(&entry.event_id))
        {
            if let Some(link) = entry
                .sent_links
                .iter_mut()
                .find(|link| link.peer_npub == peer_npub && link.link_id == link_id)
            {
                link.attempts = link.attempts.saturating_add(1);
                let delay_secs = (1_u64 << link.attempts.min(5)).min(30);
                link.retry_at = now + Duration::from_secs(delay_secs);
                continue;
            }
            while entry.sent_links.len() >= FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT {
                entry.sent_links.pop_front();
            }
            entry.sent_links.push_back(FipsNearbyOutboxLink {
                peer_npub: peer_npub.to_string(),
                link_id,
                attempts: 1,
                retry_at: now + Duration::from_secs(2),
            });
        }
    }

    pub(super) fn acknowledge(&mut self, peer_npub: &str, event_id: &str) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.event_id == event_id)
        else {
            return false;
        };
        if entry
            .recipient_targets
            .as_ref()
            .is_some_and(|targets| !targets.contains(peer_npub))
        {
            return false;
        }
        if entry.acknowledged_by(peer_npub) {
            return true;
        }
        while entry.acknowledged_peers.len() >= FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT {
            entry.acknowledged_peers.pop_front();
        }
        entry.acknowledged_peers.push_back(peer_npub.to_owned());
        true
    }

    fn acknowledged_by(&self, peer_npub: &str, event_id: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.event_id == event_id && entry.acknowledged_by(peer_npub))
    }

    // Explicit cancellation (for example group removal) still discards all copies.
    pub(super) fn forget(&mut self, event_id: &str) {
        self.entries.retain(|entry| entry.event_id != event_id);
    }
}

pub(super) fn is_fips_nearby_bootstrap_event(event: &Event) -> bool {
    event.kind == Kind::Metadata
        || is_app_keys_event(event)
        || event.kind.as_u16() as u32 == INVITE_EVENT_KIND
}

impl AppCore {
    pub(super) fn attach_host_ble(&mut self, attachment: HostBleAttachment) -> Result<(), String> {
        if attachment.ownership.is_cancelled() {
            return Err("FIPS BLE attachment was cancelled".to_string());
        }
        if self.pending_host_ble.is_some() || self.host_ble_attached {
            return Err("FIPS BLE is already attached".to_string());
        }
        self.host_ble_ownership = Some(attachment.ownership());
        self.reset_background_ble();
        self.set_host_ble_background(&attachment.ownership(), attachment.background_capable);
        self.pending_host_ble = Some(attachment);
        Ok(())
    }

    pub(super) fn detach_owned_host_ble(&mut self, ownership: &HostBleOwnership) {
        if !self
            .host_ble_ownership
            .as_ref()
            .is_some_and(|current| current.matches(ownership))
        {
            return;
        }
        self.host_ble_ownership = None;
        self.reset_background_ble();
        if self.host_ble_attached {
            self.detach_host_ble();
        } else {
            // Cancelling an attachment before bind must not restart an
            // unrelated, already-running endpoint without host BLE.
            self.pending_host_ble = None;
        }
    }

    pub(super) fn detach_host_ble(&mut self) {
        self.reset_background_ble();
        self.host_ble_ownership = None;
        self.pending_host_ble = None;
        self.stop_device_sync_now();
        self.host_ble_attached = false;
        self.reconcile_device_sync();
    }

    pub(super) fn publish_fips_nearby(&self, event: &Event) {
        if !self.private_publication_allowed(event) {
            return;
        }
        let Some(runtime) = self.device_sync.as_ref() else {
            return;
        };
        let recipient_targets = self.private_nearby_targets(event);
        if recipient_targets.is_none()
            && (!runtime.nearby_enabled
                || !self.preferences.nearby_enabled
                || !self.preferences.nearby_mailbag_enabled)
        {
            return;
        }
        let endpoint = runtime.endpoint.clone();
        let Some(payload) = encode_fips_nearby_event(event) else {
            return;
        };
        let event_id = event.id.to_string();
        let new_demand = if let Ok(mut outbox) = runtime.nearby_outbox.write() {
            let new_demand = !outbox.entries.iter().any(|entry| {
                entry.event_id == event_id && entry.recipient_targets == recipient_targets
            });
            outbox.insert_for_targets(event_id.clone(), payload.clone(), recipient_targets.clone());
            new_demand
        } else {
            false
        };
        if new_demand {
            for author in recipient_targets
                .iter()
                .flatten()
                .filter_map(|peer| PublicKey::parse(peer).ok())
                .filter(|author| {
                    !self
                        .fips_nearby_links
                        .iter()
                        .any(|link| link.device_pubkey_hex == author.to_hex())
                })
            {
                self.refresh_private_mesh_discovery_for_author(author);
            }
        }
        let outbox = runtime.nearby_outbox.clone();
        let cancelled = self.publication_task_token(&event_id);
        let local_hex = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.device_keys.public_key().to_hex())
            .unwrap_or_default();
        let configured = self
            .app_keys
            .values()
            .flat_map(|known| known.devices.iter())
            .filter(|device| device.identity_pubkey_hex != local_hex)
            .filter_map(|device| fips_peer_from_hex(&device.identity_pubkey_hex))
            .filter(|device| {
                recipient_targets
                    .as_ref()
                    .is_none_or(|targets| targets.contains(&device.npub()))
            })
            .collect::<Vec<_>>();
        self.runtime.spawn(async move {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let mut targets = configured
                .into_iter()
                .map(|peer| (peer.npub(), peer))
                .collect::<BTreeMap<_, _>>();
            if let Some(allowed) = &recipient_targets {
                for npub in allowed {
                    if let Ok(identity) = FipsPeerIdentity::from_npub(npub) {
                        targets.insert(npub.to_string(), identity);
                    }
                }
            } else if let Ok(peers) = endpoint.peers().await {
                for peer in peers.into_iter().filter(|peer| peer.connected) {
                    if let Ok(identity) = FipsPeerIdentity::from_npub(&peer.npub) {
                        targets.insert(peer.npub, identity);
                    }
                }
            }
            for (npub, target) in targets {
                if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if outbox
                    .read()
                    .map(|queue| queue.acknowledged_by(&npub, &event_id))
                    .unwrap_or(false)
                {
                    continue;
                }
                let _ = endpoint
                    .send_datagram(target, FIPS_NEARBY_PORT, FIPS_NEARBY_PORT, payload.clone())
                    .await;
            }
        });
    }

    pub(super) fn local_fips_identity_storage(&self) -> Option<SqliteStorageAdapter> {
        let login = self.logged_in.as_ref()?;
        Some(SqliteStorageAdapter::new(
            self.app_store.shared(),
            login.owner_pubkey.to_hex(),
            login.device_keys.public_key().to_hex(),
        ))
    }

    pub(super) fn cached_local_fips_identity(&self, kind: Kind) -> Option<Event> {
        let login = self.logged_in.as_ref()?;
        let key = format!("appcore/nearby-identity-v1/{}", kind.as_u16());
        let json = self.local_fips_identity_storage()?.get(&key).ok()??;
        let event: Event = serde_json::from_str(&json).ok()?;
        (event.pubkey == login.owner_pubkey && event.kind == kind && event.verify().is_ok())
            .then_some(event)
    }

    pub(super) fn cache_local_fips_identity(&mut self, event: &Event) {
        if self
            .logged_in
            .as_ref()
            .is_none_or(|login| login.owner_pubkey != event.pubkey)
            || !(event.kind == Kind::Metadata || is_app_keys_event(event))
            || event.verify().is_err()
        {
            return;
        }
        if event.kind == Kind::Metadata {
            if !serde_json::from_str::<serde_json::Value>(&event.content)
                .is_ok_and(|value| value.is_object())
            {
                return;
            }
        } else if !app_keys_event_is_acceptable_for_owner(event.pubkey, event, unix_now().get()) {
            return;
        }
        if self
            .cached_local_fips_identity(event.kind)
            .is_some_and(|current| {
                current.created_at > event.created_at
                    || (current.created_at == event.created_at && current.id <= event.id)
            })
        {
            return;
        }
        let Some(storage) = self.local_fips_identity_storage() else {
            return;
        };
        let key = format!("appcore/nearby-identity-v1/{}", event.kind.as_u16());
        let result = serde_json::to_string(event)
            .map_err(anyhow::Error::from)
            .and_then(|json| storage.put(&key, json).map_err(anyhow::Error::from));
        match result {
            Ok(()) => self.refresh_fips_nearby_bootstrap(),
            Err(error) => self.push_debug_log("fips_nearby.identity_cache", error.to_string()),
        }
    }

    fn deferred_owner_app_keys_for_fips_nearby(&self) -> Option<Event> {
        if !self.defer_owner_app_keys_publish {
            return None;
        }
        let logged_in = self.logged_in.as_ref()?;
        let owner_keys = logged_in.owner_keys.as_ref()?;
        let known = self.app_keys.get(&logged_in.owner_pubkey.to_hex());
        let created_at = known
            .map(|app_keys| app_keys.created_at_secs)
            .filter(|created_at| *created_at > 0)
            .unwrap_or(1);
        let mut app_keys = known
            .map(known_app_keys_to_ndr)
            .unwrap_or_else(|| AppKeys::new(Vec::new()));
        let device_pubkey = logged_in.device_keys.public_key();
        if app_keys.get_device(&device_pubkey).is_none() {
            app_keys.add_device(DeviceEntry::new(device_pubkey, created_at));
        }
        if let Some(labels) = self.current_device_labels.as_ref() {
            app_keys.set_device_labels(
                device_pubkey,
                labels.device_label.clone(),
                labels.client_label.clone(),
                Some(created_at),
            );
        }
        self.signed_local_app_keys_snapshot(&app_keys, created_at, owner_keys)
            .ok()
    }

    pub(super) fn refresh_fips_nearby_bootstrap(&self) {
        let Some(runtime) = self.device_sync.as_ref() else {
            return;
        };
        if let Ok(mut payloads) = runtime.nearby_bootstrap_payloads.write() {
            *payloads = self.local_fips_nearby_bootstrap_payloads();
        }
    }

    pub(super) fn handle_fips_nearby_packet(
        &mut self,
        source_pubkey_hex: &str,
        source_port: u16,
        data: &[u8],
    ) {
        let packet = match FipsNearbyPacket::decode_diagnostic(data) {
            Ok(packet) => packet,
            Err(reason) => {
                self.record_fips_nearby_outcome(reason, None, data.len());
                return;
            }
        };
        match packet {
            FipsNearbyPacket::Event {
                event_id,
                event_json,
                ..
            } => {
                let Ok(event) = serde_json::from_str::<Event>(&event_json) else {
                    self.record_fips_nearby_outcome("event_json", None, data.len());
                    return;
                };
                let kind = Some(event.kind.as_u16());
                if event.id.to_string() != event_id {
                    self.record_fips_nearby_outcome("event_id", kind, data.len());
                    return;
                }
                if !self.accepts_private_nearby_event(&event) {
                    self.record_fips_nearby_outcome("recipient", kind, data.len());
                    return;
                }
                // Only the opt-in diagnostic lane computes this extra hash.
                // Authentication and application effects retain their existing path.
                let id_valid = debug_snapshot_file_writes_enabled().then(|| event.verify_id());
                if !self.handle_relay_event_with_channel(event, "FIPS nearby") {
                    let reason = if id_valid == Some(false) {
                        "event_hash"
                    } else {
                        "signature"
                    };
                    self.record_fips_nearby_outcome(reason, kind, data.len());
                    return;
                }
                self.record_fips_nearby_outcome("accepted", kind, data.len());
                let Some(source) = fips_peer_from_hex(source_pubkey_hex) else {
                    self.record_fips_nearby_outcome("receipt_source", kind, data.len());
                    return;
                };
                let Some(payload) =
                    FipsNearbyPacket::receipt(event_id).and_then(|value| value.encode())
                else {
                    return;
                };
                let Some(endpoint) = self
                    .device_sync
                    .as_ref()
                    .map(|runtime| runtime.endpoint.clone())
                else {
                    return;
                };
                self.runtime.spawn(async move {
                    let _ = endpoint
                        .send_datagram(source, FIPS_NEARBY_PORT, source_port, payload)
                        .await;
                });
            }
            FipsNearbyPacket::Receipt { event_id, .. } => {
                let Some(source) = fips_peer_from_hex(source_pubkey_hex) else {
                    self.record_fips_nearby_outcome("receipt_source", None, data.len());
                    return;
                };
                let acknowledged = self.device_sync.as_ref().is_some_and(|runtime| {
                    runtime
                        .nearby_outbox
                        .write()
                        .map(|mut outbox| outbox.acknowledge(&source.npub(), &event_id))
                        .unwrap_or(false)
                });
                if !acknowledged {
                    self.record_fips_nearby_outcome("receipt_unmatched", None, data.len());
                    return;
                }
                self.record_fips_nearby_outcome("receipt_matched", None, data.len());
                let changed = self.add_transport_channel_for_event_id(&event_id, "FIPS nearby");
                self.push_debug_log(
                    "fips_nearby.receipt",
                    format!(
                        "event={event_id} source={source_pubkey_hex} matched_message={changed}"
                    ),
                );
                if changed {
                    self.persist_best_effort();
                    self.rebuild_state();
                    self.emit_state();
                }
            }
        }
    }

    pub(super) fn update_fips_connection_links(
        &mut self,
        links: Vec<crate::updates::FipsNearbyLinkSnapshot>,
    ) {
        if self.fips_nearby_links == links {
            return;
        }
        // These are direct transport links. End-to-end history streams can keep
        // using a routed path when one disappears; their requests and revocation
        // checks own reconciliation cancellation.
        self.fips_nearby_links = links;
        self.emit_fips_nearby_peers();
        let roster = self.build_device_roster_snapshot();
        if self.state.device_roster != roster {
            self.state.device_roster = roster;
            self.emit_state();
        }
    }

    pub(super) fn emit_fips_nearby_peers(&self) {
        let now = unix_now().get();
        let local_device = self
            .logged_in
            .as_ref()
            .map(|login| login.device_keys.public_key().to_hex());
        let own_devices = self
            .logged_in
            .as_ref()
            .and_then(|login| self.app_keys.get(&login.owner_pubkey.to_hex()));
        let mut bluetooth_peer_ids = Vec::new();
        let mut lan_peer_ids = Vec::new();
        let peers = self
            .fips_nearby_links
            .iter()
            .filter_map(|link| {
                if local_device
                    .as_ref()
                    .is_some_and(|device| device.eq_ignore_ascii_case(&link.device_pubkey_hex))
                    || own_devices.is_some_and(|keys| {
                        keys.devices.iter().any(|device| {
                            device
                                .identity_pubkey_hex
                                .eq_ignore_ascii_case(&link.device_pubkey_hex)
                        })
                    })
                {
                    // Keep our device links for sync, but don't present them as other people.
                    return None;
                }
                let id = link.device_pubkey_hex.clone();
                let transport = link.transport_type.to_ascii_lowercase();
                if transport.contains("ble") || transport.contains("bluetooth") {
                    bluetooth_peer_ids.push(id.clone());
                } else if transport.contains("ethernet")
                    || (transport == "udp" && is_local_udp_address(link.transport_addr.as_deref()))
                {
                    lan_peer_ids.push(id.clone());
                } else {
                    // Transit connections keep the mesh connected, but are not nearby users.
                    return None;
                }
                let owner_pubkey_hex = self.app_keys.iter().find_map(|(owner, app_keys)| {
                    app_keys
                        .devices
                        .iter()
                        .any(|device| {
                            device
                                .identity_pubkey_hex
                                .eq_ignore_ascii_case(&link.device_pubkey_hex)
                        })
                        .then(|| owner.clone())
                });
                let name = owner_pubkey_hex
                    .as_deref()
                    .map(|owner| self.owner_display_label(owner))
                    .unwrap_or_else(|| "Nearby user".to_string());
                let picture_url = owner_pubkey_hex
                    .as_deref()
                    .and_then(|owner| self.owner_picture_url(owner));
                Some(DesktopNearbyPeerSnapshot {
                    id,
                    name,
                    owner_pubkey_hex,
                    picture_url,
                    profile_event_id: None,
                    last_seen_secs: now,
                })
            })
            .collect::<Vec<_>>();
        let visible = self.preferences.nearby_enabled
            && (self.preferences.nearby_bluetooth_enabled || self.preferences.nearby_lan_enabled);
        let _ = self.update_tx.send(AppUpdate::NearbyPeersChanged {
            snapshot: DesktopNearbySnapshot {
                visible,
                status: if visible { "Visible" } else { "Off" }.to_string(),
                peers,
            },
            bluetooth_peer_ids,
            lan_peer_ids,
        });
    }
}

fn is_local_udp_address(address: Option<&str>) -> bool {
    let Some(address) = address.and_then(|value| value.parse::<std::net::SocketAddr>().ok()) else {
        return false;
    };
    match address.ip() {
        std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        std::net::IpAddr::V6(ip) => ip.is_unicast_link_local() || ip.is_unique_local(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ownership_tests;

mod bootstrap;
pub(super) use bootstrap::FipsNearbyBootstrapCache;
