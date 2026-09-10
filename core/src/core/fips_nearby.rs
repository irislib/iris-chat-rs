use super::*;
use crate::{DesktopNearbyPeerSnapshot, DesktopNearbySnapshot};
use fips_core::transport::ble::host::HostBleIo;
use fips_core::PeerIdentity as FipsPeerIdentity;

pub(super) const FIPS_NEARBY_PORT: u16 = 7370;
pub(super) const FIPS_NEARBY_SCOPE: &str = "iris-chat-nearby-v1";
const FIPS_NEARBY_VERSION: u8 = 1;
const FIPS_NEARBY_MAX_PACKET_BYTES: usize = 48 * 1024;
pub(super) const FIPS_NEARBY_OUTBOX_MAX_EVENTS: usize = 256;
const FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT: usize = 32;

pub(crate) struct HostBleAttachment(Option<HostBleIo>);

impl HostBleAttachment {
    pub(crate) fn new(io: HostBleIo) -> Self {
        Self(Some(io))
    }

    pub(super) fn take(&mut self) -> Option<HostBleIo> {
        self.0.take()
    }
}

impl std::fmt::Debug for HostBleAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostBleAttachment")
            .field("available", &self.0.is_some())
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

    pub(super) fn decode(data: &[u8]) -> Option<Self> {
        if data.len() > FIPS_NEARBY_MAX_PACKET_BYTES {
            return None;
        }
        let packet = serde_json::from_slice::<Self>(data).ok()?;
        match &packet {
            Self::Event {
                v,
                event_id,
                event_json,
            } => (*v == FIPS_NEARBY_VERSION
                && valid_event_id(event_id)
                && event_json.len() <= FIPS_NEARBY_MAX_PACKET_BYTES)
                .then_some(packet),
            Self::Receipt { v, event_id } => {
                (*v == FIPS_NEARBY_VERSION && valid_event_id(event_id)).then_some(packet)
            }
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
}

#[derive(Clone, Debug)]
struct FipsNearbyOutboxEntry {
    event_id: String,
    payload: Vec<u8>,
    sent_links: VecDeque<FipsNearbyOutboxLink>,
}

/// Bounded nearby-event queue shared with the FIPS link monitor.
///
/// A successful send suppresses repeats only for that exact logical link.
/// When a peer reconnects with a new link id, unacknowledged events become
/// eligible again. Signed FIPS-nearby receipts remove them from the queue.
#[derive(Default, Debug)]
pub(super) struct FipsNearbyOutbox {
    entries: VecDeque<FipsNearbyOutboxEntry>,
}

impl FipsNearbyOutbox {
    pub(super) fn insert(&mut self, event_id: String, payload: Vec<u8>) {
        let sent_links = self
            .entries
            .iter()
            .position(|entry| entry.event_id == event_id)
            .and_then(|index| self.entries.remove(index))
            .map(|entry| entry.sent_links)
            .unwrap_or_default();
        while self.entries.len() >= FIPS_NEARBY_OUTBOX_MAX_EVENTS {
            self.entries.pop_front();
        }
        self.entries.push_back(FipsNearbyOutboxEntry {
            event_id,
            payload,
            sent_links,
        });
    }

    pub(super) fn pending_for_link(&self, peer_npub: &str, link_id: u64) -> Vec<(String, Vec<u8>)> {
        self.entries
            .iter()
            .filter(|entry| {
                !entry
                    .sent_links
                    .iter()
                    .any(|link| link.peer_npub == peer_npub && link.link_id == link_id)
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
        let link = FipsNearbyOutboxLink {
            peer_npub: peer_npub.to_string(),
            link_id,
        };
        for entry in self
            .entries
            .iter_mut()
            .filter(|entry| event_ids.contains(&entry.event_id))
        {
            if entry.sent_links.contains(&link) {
                continue;
            }
            while entry.sent_links.len() >= FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT {
                entry.sent_links.pop_front();
            }
            entry.sent_links.push_back(link.clone());
        }
    }

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
        if self.pending_host_ble.is_some() || self.host_ble_attached {
            return Err("FIPS BLE is already attached".to_string());
        }
        self.pending_host_ble = Some(attachment);
        Ok(())
    }

    pub(super) fn detach_host_ble(&mut self) {
        self.pending_host_ble = None;
        self.stop_device_sync_now();
        self.host_ble_attached = false;
        self.reconcile_device_sync();
    }

    pub(super) fn publish_fips_nearby(&self, event: &Event) {
        if !self.preferences.nearby_enabled || !self.preferences.nearby_mailbag_enabled {
            return;
        }
        let Some(runtime) = self.device_sync.as_ref() else {
            return;
        };
        if !runtime.nearby_enabled {
            return;
        }
        let endpoint = runtime.endpoint.clone();
        let Some(payload) = encode_fips_nearby_event(event) else {
            return;
        };
        if let Ok(mut outbox) = runtime.nearby_outbox.write() {
            outbox.insert(event.id.to_string(), payload.clone());
        }
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
            .collect::<Vec<_>>();
        self.runtime.spawn(async move {
            let mut targets = configured
                .into_iter()
                .map(|peer| (peer.npub(), peer))
                .collect::<BTreeMap<_, _>>();
            if let Ok(peers) = endpoint.peers().await {
                for peer in peers.into_iter().filter(|peer| peer.connected) {
                    if let Ok(identity) = FipsPeerIdentity::from_npub(&peer.npub) {
                        targets.insert(peer.npub, identity);
                    }
                }
            }
            for target in targets.into_values() {
                let _ = endpoint
                    .send_datagram(target, FIPS_NEARBY_PORT, FIPS_NEARBY_PORT, payload.clone())
                    .await;
            }
        });
    }

    pub(super) fn local_fips_nearby_bootstrap_payloads(&self) -> Vec<Vec<u8>> {
        let (background, mut durable) = self.build_local_identity_artifacts();
        if let Some(event) = self.deferred_owner_app_keys_for_fips_nearby() {
            durable.insert(0, ("app-keys-nearby", event));
        }
        durable
            .into_iter()
            .chain(background)
            .map(|(_, event)| event)
            .filter(is_fips_nearby_bootstrap_event)
            .filter_map(|event| encode_fips_nearby_event(&event))
            .collect()
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
        app_keys
            .get_encrypted_event_at(owner_keys, created_at)
            .ok()?
            .sign_with_keys(owner_keys)
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
        let Some(packet) = FipsNearbyPacket::decode(data) else {
            return;
        };
        match packet {
            FipsNearbyPacket::Event {
                event_id,
                event_json,
                ..
            } => {
                let Ok(event) = serde_json::from_str::<Event>(&event_json) else {
                    return;
                };
                if event.id.to_string() != event_id || event.verify().is_err() {
                    return;
                }
                self.handle_relay_event_with_channel(event, "FIPS nearby");
                self.emit_fips_nearby_peers();
                let Some(source) = fips_peer_from_hex(source_pubkey_hex) else {
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
                if let Some(runtime) = self.device_sync.as_ref() {
                    if let Ok(mut outbox) = runtime.nearby_outbox.write() {
                        outbox.forget(&event_id);
                    }
                }
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

    pub(super) fn emit_fips_nearby_peers(&self) {
        let now = unix_now().get();
        let mut bluetooth_peer_ids = Vec::new();
        let mut lan_peer_ids = Vec::new();
        let peers = self
            .fips_nearby_links
            .iter()
            .filter_map(|link| {
                let id = link.device_pubkey_hex.clone();
                let transport = link.transport_type.to_ascii_lowercase();
                if transport.contains("ble") || transport.contains("bluetooth") {
                    bluetooth_peer_ids.push(id.clone());
                } else if transport.contains("udp") || transport.contains("ethernet") {
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

#[cfg(test)]
mod tests;
