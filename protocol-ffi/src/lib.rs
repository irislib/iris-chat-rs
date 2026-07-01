use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use iris_chat_protocol::{
    invite_unsigned_event, invite_url, is_app_keys_event, parse_invite_event,
    parse_invite_response_event, parse_invite_url, parse_message_event, AppKeys, DeviceEntry,
    FileStorageAdapter, InMemoryStorage, NdrUnixSeconds, ProtocolDecryptedMessage, ProtocolEffect,
    ProtocolEngine, ProtocolRetryBatch, StorageAdapter, UnixSeconds, APP_KEYS_EVENT_KIND,
    INVITE_EVENT_KIND, INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND,
};
use nostr::{Event, Filter, Keys, Kind, PublicKey, SecretKey};

mod error;
pub use error::NdrError;

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct FfiKeyPair {
    pub public_key_hex: String,
    pub private_key_hex: String,
}

#[uniffi::export]
pub fn generate_keypair() -> FfiKeyPair {
    let keys = Keys::generate();
    FfiKeyPair {
        public_key_hex: keys.public_key().to_hex(),
        private_key_hex: keys.secret_key().to_secret_hex(),
    }
}

#[uniffi::export]
pub fn derive_public_key(privkey_hex: String) -> Result<String, NdrError> {
    Ok(keys_from_private_hex(&privkey_hex)?.public_key().to_hex())
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct PubSubEvent {
    pub kind: String,
    pub subid: Option<String>,
    pub filter_json: Option<String>,
    pub event_json: Option<String>,
    pub sender_pubkey_hex: Option<String>,
    pub content: Option<String>,
    pub event_id: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct SessionManagerAcceptInviteResult {
    pub owner_pubkey_hex: String,
    pub inviter_device_pubkey_hex: String,
    pub device_id: String,
    pub created_new_session: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct SendTextResult {
    pub inner_id: String,
    pub outer_event_ids: Vec<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct MessagePushSessionStateResult {
    pub state_json: String,
    pub tracked_sender_pubkeys: Vec<String>,
    pub has_receiving_capability: bool,
}

#[derive(uniffi::Object)]
pub struct InviteHandle {
    inner: Mutex<iris_chat_protocol::Invite>,
}

#[uniffi::export]
impl InviteHandle {
    #[uniffi::constructor]
    pub fn from_url(url: String) -> Result<Arc<Self>, NdrError> {
        let invite = parse_invite_url(&url).map_err(invalid_event_error)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(invite),
        }))
    }

    #[uniffi::constructor]
    pub fn from_event_json(event_json: String) -> Result<Arc<Self>, NdrError> {
        let event: Event = serde_json::from_str(&event_json)?;
        let invite = parse_invite_event(&event).map_err(invalid_event_error)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(invite),
        }))
    }

    pub fn to_url(&self, root: String) -> Result<String, NdrError> {
        let invite = self
            .inner
            .lock()
            .map_err(|_| NdrError::StateMismatch("invite mutex poisoned".to_string()))?;
        Ok(invite_url(&invite, &root).map_err(invalid_event_error)?)
    }

    pub fn get_inviter_pubkey_hex(&self) -> String {
        self.inner
            .lock()
            .map(|invite| invite.inviter.to_hex())
            .unwrap_or_default()
    }
}

#[derive(uniffi::Object)]
pub struct SessionManagerHandle {
    inner: Mutex<SessionManagerInner>,
}

struct SessionManagerInner {
    owner_pubkey: PublicKey,
    device_keys: Keys,
    device_id: String,
    storage: Arc<dyn StorageAdapter>,
    engine: Option<ProtocolEngine>,
    events: VecDeque<PubSubEvent>,
    message_subid: Option<String>,
    message_author_hexes: Vec<String>,
}

#[uniffi::export]
impl SessionManagerHandle {
    #[uniffi::constructor]
    pub fn new(
        our_pubkey_hex: String,
        our_identity_privkey_hex: String,
        device_id: String,
        owner_pubkey_hex: Option<String>,
    ) -> Result<Arc<Self>, NdrError> {
        let device_keys = keys_from_private_hex(&our_identity_privkey_hex)?;
        validate_device_pubkey(&our_pubkey_hex, &device_keys)?;
        let owner_pubkey = owner_pubkey_hex
            .as_deref()
            .map(parse_pubkey)
            .transpose()?
            .unwrap_or_else(|| device_keys.public_key());
        let storage: Arc<dyn StorageAdapter> = Arc::new(InMemoryStorage::new());
        Ok(Arc::new(Self {
            inner: Mutex::new(SessionManagerInner {
                owner_pubkey,
                device_keys,
                device_id,
                storage,
                engine: None,
                events: VecDeque::new(),
                message_subid: None,
                message_author_hexes: Vec::new(),
            }),
        }))
    }

    #[uniffi::constructor]
    pub fn new_with_storage_path(
        our_pubkey_hex: String,
        our_identity_privkey_hex: String,
        device_id: String,
        storage_path: String,
        owner_pubkey_hex: Option<String>,
    ) -> Result<Arc<Self>, NdrError> {
        let device_keys = keys_from_private_hex(&our_identity_privkey_hex)?;
        validate_device_pubkey(&our_pubkey_hex, &device_keys)?;
        let owner_pubkey = owner_pubkey_hex
            .as_deref()
            .map(parse_pubkey)
            .transpose()?
            .unwrap_or_else(|| device_keys.public_key());
        let storage: Arc<dyn StorageAdapter> =
            Arc::new(FileStorageAdapter::new(PathBuf::from(storage_path))?);
        Ok(Arc::new(Self {
            inner: Mutex::new(SessionManagerInner {
                owner_pubkey,
                device_keys,
                device_id,
                storage,
                engine: None,
                events: VecDeque::new(),
                message_subid: None,
                message_author_hexes: Vec::new(),
            }),
        }))
    }

    pub fn init(&self) -> Result<(), NdrError> {
        let mut inner = self.lock_inner()?;
        if inner.engine.is_some() {
            return Ok(());
        }
        let mut engine = ProtocolEngine::load_or_create_for_local_device(
            Arc::clone(&inner.storage),
            inner.owner_pubkey,
            &inner.device_keys,
        )?;
        publish_startup_state(&mut inner, &mut engine)?;
        inner.engine = Some(engine);
        inner.sync_message_subscription()?;
        Ok(())
    }

    pub fn setup_user(&self, user_pubkey_hex: String) -> Result<(), NdrError> {
        let user_pubkey = parse_pubkey(&user_pubkey_hex)?;
        let mut inner = self.lock_inner()?;
        let engine = inner.engine_mut()?;
        let effects = engine.protocol_discovery_effects_for_owners(
            [user_pubkey],
            UnixSeconds(now_secs()),
            "ffi_setup_user",
        );
        inner.enqueue_effects(effects)?;
        inner.sync_message_subscription()?;
        Ok(())
    }

    pub fn accept_invite_from_url(
        &self,
        invite_url: String,
        owner_pubkey_hint_hex: Option<String>,
    ) -> Result<SessionManagerAcceptInviteResult, NdrError> {
        let invite = parse_invite_url(&invite_url).map_err(invalid_event_error)?;
        self.accept_invite(invite, owner_pubkey_hint_hex)
    }

    pub fn accept_invite_from_event_json(
        &self,
        event_json: String,
        owner_pubkey_hint_hex: Option<String>,
    ) -> Result<SessionManagerAcceptInviteResult, NdrError> {
        let event: Event = serde_json::from_str(&event_json)?;
        let invite = parse_invite_event(&event).map_err(invalid_event_error)?;
        self.accept_invite(invite, owner_pubkey_hint_hex)
    }

    pub fn send_text(
        &self,
        recipient_pubkey_hex: String,
        text: String,
        expires_at_seconds: Option<u64>,
    ) -> Result<Vec<String>, NdrError> {
        let recipient = parse_pubkey(&recipient_pubkey_hex)?;
        let mut inner = self.lock_inner()?;
        let engine = inner.engine_mut()?;
        let result = engine.send_direct_text(
            recipient,
            &recipient.to_hex(),
            &text,
            expires_at_seconds,
            UnixSeconds(now_secs()),
        )?;
        let event_ids = result.event_ids.clone();
        inner.enqueue_effects(result.effects)?;
        inner.sync_message_subscription()?;
        Ok(event_ids)
    }

    pub fn send_text_with_inner_id(
        &self,
        recipient_pubkey_hex: String,
        text: String,
        expires_at_seconds: Option<u64>,
    ) -> Result<SendTextResult, NdrError> {
        let recipient = parse_pubkey(&recipient_pubkey_hex)?;
        let mut inner = self.lock_inner()?;
        let engine = inner.engine_mut()?;
        let result = engine.send_direct_text(
            recipient,
            &recipient.to_hex(),
            &text,
            expires_at_seconds,
            UnixSeconds(now_secs()),
        )?;
        let output = SendTextResult {
            inner_id: result.message_id,
            outer_event_ids: result.event_ids.clone(),
        };
        inner.enqueue_effects(result.effects)?;
        inner.sync_message_subscription()?;
        Ok(output)
    }

    pub fn process_event(&self, event_json: String) -> Result<(), NdrError> {
        let event: Event = serde_json::from_str(&event_json)?;
        let mut inner = self.lock_inner()?;
        process_event_inner(&mut inner, event)?;
        inner.sync_message_subscription()?;
        Ok(())
    }

    pub fn drain_events(&self) -> Result<Vec<PubSubEvent>, NdrError> {
        let mut inner = self.lock_inner()?;
        let events: Vec<_> = inner.events.drain(..).collect();
        if events.iter().any(|event| event.kind == "decrypted_message") {
            if let Some(engine) = inner.engine.as_mut() {
                engine.ack_pending_decrypted_deliveries()?;
            }
        }
        Ok(events)
    }

    pub fn get_active_session_state(
        &self,
        peer_pubkey_hex: String,
    ) -> Result<Option<String>, NdrError> {
        let peer = parse_pubkey(&peer_pubkey_hex)?;
        let inner = self.lock_inner()?;
        let Some(engine) = inner.engine.as_ref() else {
            return Ok(None);
        };
        let snapshot = engine.session_manager_snapshot();
        let peer_hex = peer.to_hex();
        for user in snapshot.users {
            if user.owner_pubkey.to_string() != peer_hex {
                continue;
            }
            for device in user.devices {
                if let Some(state) = device.active_session {
                    return Ok(Some(serde_json::to_string(&state)?));
                }
            }
        }
        Ok(None)
    }

    pub fn known_peer_owner_pubkeys(&self) -> Vec<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| {
                inner.engine.as_ref().map(|engine| {
                    let local = inner.owner_pubkey.to_hex();
                    let mut owners = engine
                        .session_manager_snapshot()
                        .users
                        .into_iter()
                        .map(|user| user.owner_pubkey.to_string())
                        .filter(|owner| owner != &local)
                        .collect::<Vec<_>>();
                    owners.sort();
                    owners.dedup();
                    owners
                })
            })
            .unwrap_or_default()
    }

    pub fn get_message_push_author_pubkeys(
        &self,
        peer_owner_pubkey_hex: String,
    ) -> Result<Vec<String>, NdrError> {
        let peer = parse_pubkey(&peer_owner_pubkey_hex)?;
        let inner = self.lock_inner()?;
        let Some(engine) = inner.engine.as_ref() else {
            return Ok(Vec::new());
        };
        Ok(engine
            .message_author_pubkeys_for_owner(peer)
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect())
    }

    pub fn get_message_push_session_states(
        &self,
        peer_owner_pubkey_hex: String,
    ) -> Result<Vec<MessagePushSessionStateResult>, NdrError> {
        let peer = parse_pubkey(&peer_owner_pubkey_hex)?;
        let inner = self.lock_inner()?;
        let Some(engine) = inner.engine.as_ref() else {
            return Ok(Vec::new());
        };
        let snapshot = engine.session_manager_snapshot();
        ProtocolEngine::message_session_debug_snapshots_with_snapshot(&snapshot, peer)
            .into_iter()
            .map(|state| {
                Ok(MessagePushSessionStateResult {
                    state_json: serde_json::to_string(&state.state)?,
                    tracked_sender_pubkeys: state
                        .tracked_sender_pubkeys
                        .into_iter()
                        .map(|pubkey| pubkey.to_hex())
                        .collect(),
                    has_receiving_capability: state.has_receiving_capability,
                })
            })
            .collect()
    }

    pub fn get_device_id(&self) -> String {
        self.inner
            .lock()
            .map(|inner| inner.device_id.clone())
            .unwrap_or_default()
    }

    pub fn get_our_pubkey_hex(&self) -> String {
        self.inner
            .lock()
            .map(|inner| inner.device_keys.public_key().to_hex())
            .unwrap_or_default()
    }

    pub fn get_owner_pubkey_hex(&self) -> String {
        self.inner
            .lock()
            .map(|inner| inner.owner_pubkey.to_hex())
            .unwrap_or_default()
    }

    pub fn get_total_sessions(&self) -> u64 {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| {
                inner.engine.as_ref().map(|engine| {
                    engine
                        .session_manager_snapshot()
                        .users
                        .iter()
                        .flat_map(|user| user.devices.iter())
                        .map(|device| {
                            u64::from(device.active_session.is_some())
                                + device.inactive_sessions.len() as u64
                        })
                        .sum()
                })
            })
            .unwrap_or_default()
    }
}

impl SessionManagerHandle {
    fn accept_invite(
        &self,
        invite: iris_chat_protocol::Invite,
        owner_pubkey_hint_hex: Option<String>,
    ) -> Result<SessionManagerAcceptInviteResult, NdrError> {
        let owner_hint = owner_pubkey_hint_hex
            .as_deref()
            .map(parse_pubkey)
            .transpose()?;
        let mut inner = self.lock_inner()?;
        let engine = inner.engine_mut()?;
        let candidate_owner = invite_owner_candidate(&invite, owner_hint);
        let active_before = engine.active_session_count_for_owner(candidate_owner);
        let result = engine.accept_invite(&invite, owner_hint)?;
        let active_after = engine.active_session_count_for_owner(result.owner_pubkey);
        let output = SessionManagerAcceptInviteResult {
            owner_pubkey_hex: result.owner_pubkey.to_hex(),
            inviter_device_pubkey_hex: result.inviter_device_pubkey.to_hex(),
            device_id: result.device_id,
            created_new_session: active_after > active_before,
        };
        inner.enqueue_effects(result.effects)?;
        inner.sync_message_subscription()?;
        Ok(output)
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, SessionManagerInner>, NdrError> {
        self.inner
            .lock()
            .map_err(|_| NdrError::StateMismatch("session manager mutex poisoned".to_string()))
    }
}

impl SessionManagerInner {
    fn engine_mut(&mut self) -> Result<&mut ProtocolEngine, NdrError> {
        self.engine
            .as_mut()
            .ok_or_else(|| NdrError::SessionNotReady("session manager not initialized".to_string()))
    }

    fn enqueue_effects(&mut self, effects: Vec<ProtocolEffect>) -> Result<(), NdrError> {
        for effect in effects {
            self.enqueue_effect(effect)?;
        }
        Ok(())
    }

    fn enqueue_retry_batch(&mut self, batch: ProtocolRetryBatch) -> Result<(), NdrError> {
        for result in batch.direct_results {
            self.enqueue_effects(result.effects)?;
        }
        self.enqueue_effects(batch.group_result.effects)?;
        self.enqueue_effects(batch.effects)?;
        for message in batch.direct_messages {
            self.enqueue_decrypted_message(message);
        }
        Ok(())
    }

    fn enqueue_effect(&mut self, effect: ProtocolEffect) -> Result<(), NdrError> {
        match effect {
            ProtocolEffect::Publish(publish) => {
                self.events.push_back(PubSubEvent {
                    kind: "publish_signed".to_string(),
                    subid: None,
                    filter_json: None,
                    event_json: Some(serde_json::to_string(&publish.event)?),
                    sender_pubkey_hex: None,
                    content: None,
                    event_id: publish.inner_event_id,
                });
            }
            ProtocolEffect::FetchProtocolState { filters, reason } => {
                for filter in filters {
                    self.enqueue_subscribe_filter(reason, filter)?;
                }
            }
        }
        Ok(())
    }

    fn enqueue_decrypted_message(&mut self, message: ProtocolDecryptedMessage) {
        if let Some(event_id) = message.event_id.as_deref() {
            if self.events.iter().any(|event| {
                event.kind == "decrypted_message" && event.event_id.as_deref() == Some(event_id)
            }) {
                return;
            }
        }
        self.events.push_back(PubSubEvent {
            kind: "decrypted_message".to_string(),
            subid: None,
            filter_json: None,
            event_json: None,
            sender_pubkey_hex: Some(message.sender.to_hex()),
            content: Some(message.content),
            event_id: message.event_id,
        });
    }

    fn enqueue_subscribe_filter(&mut self, reason: &str, filter: Filter) -> Result<(), NdrError> {
        self.events.push_back(PubSubEvent {
            kind: "subscribe".to_string(),
            subid: Some(format!("icp-{reason}-{}", uuid::Uuid::new_v4())),
            filter_json: Some(serde_json::to_string(&filter)?),
            event_json: None,
            sender_pubkey_hex: None,
            content: None,
            event_id: None,
        });
        Ok(())
    }

    fn sync_message_subscription(&mut self) -> Result<(), NdrError> {
        let Some(engine) = self.engine.as_ref() else {
            return Ok(());
        };
        let mut authors = engine.known_message_author_pubkeys();
        authors.sort_by_key(|pubkey| pubkey.to_hex());
        authors.dedup();
        let author_hexes = authors
            .iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<Vec<_>>();
        if author_hexes == self.message_author_hexes {
            return Ok(());
        }
        if let Some(subid) = self.message_subid.take() {
            self.events.push_back(PubSubEvent {
                kind: "unsubscribe".to_string(),
                subid: Some(subid),
                filter_json: None,
                event_json: None,
                sender_pubkey_hex: None,
                content: None,
                event_id: None,
            });
        }
        self.message_author_hexes = author_hexes;
        if authors.is_empty() {
            return Ok(());
        }
        let subid = "icp-messages".to_string();
        let filter = Filter::new()
            .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
            .authors(authors);
        self.events.push_back(PubSubEvent {
            kind: "subscribe".to_string(),
            subid: Some(subid.clone()),
            filter_json: Some(serde_json::to_string(&filter)?),
            event_json: None,
            sender_pubkey_hex: None,
            content: None,
            event_id: None,
        });
        self.message_subid = Some(subid);
        Ok(())
    }
}

fn publish_startup_state(
    inner: &mut SessionManagerInner,
    engine: &mut ProtocolEngine,
) -> Result<(), NdrError> {
    let now = now_secs();
    if inner.device_keys.public_key() == inner.owner_pubkey {
        let app_keys = AppKeys::new(vec![DeviceEntry::new(inner.device_keys.public_key(), now)]);
        let unsigned = app_keys
            .get_encrypted_event_at(&inner.device_keys, now)
            .map_err(invalid_event_error)?;
        let signed = unsigned.sign_with_keys(&inner.device_keys)?;
        inner.events.push_back(PubSubEvent {
            kind: "publish_signed".to_string(),
            subid: None,
            filter_json: None,
            event_json: Some(serde_json::to_string(&signed)?),
            sender_pubkey_hex: None,
            content: None,
            event_id: None,
        });
        let retry = engine.ingest_app_keys_snapshot(inner.owner_pubkey, app_keys, now)?;
        inner.enqueue_retry_batch(retry)?;
    }

    if let Some(invite) = engine.local_invite() {
        let unsigned = invite_unsigned_event(&invite).map_err(invalid_event_error)?;
        let signed = unsigned.sign_with_keys(&inner.device_keys)?;
        inner.events.push_back(PubSubEvent {
            kind: "publish_signed".to_string(),
            subid: None,
            filter_json: None,
            event_json: Some(serde_json::to_string(&signed)?),
            sender_pubkey_hex: None,
            content: None,
            event_id: None,
        });
    }

    if let Some(response_pubkey) = engine.local_invite_response_pubkey() {
        let filter = Filter::new()
            .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
            .pubkey(response_pubkey);
        inner.enqueue_subscribe_filter("invite-response", filter)?;
    }

    Ok(())
}

fn process_event_inner(inner: &mut SessionManagerInner, event: Event) -> Result<(), NdrError> {
    let kind = event.kind.as_u16() as u32;
    let engine = inner.engine_mut()?;
    if kind == APP_KEYS_EVENT_KIND && is_app_keys_event(&event) {
        let app_keys = AppKeys::from_event(&event).map_err(invalid_event_error)?;
        let retry =
            engine.ingest_app_keys_snapshot(event.pubkey, app_keys, event.created_at.as_secs())?;
        inner.enqueue_retry_batch(retry)?;
        return Ok(());
    }
    if kind == INVITE_EVENT_KIND {
        let retry = engine.observe_invite_event(&event)?;
        inner.enqueue_retry_batch(retry)?;
        return Ok(());
    }
    if kind == INVITE_RESPONSE_KIND && parse_invite_response_event(&event).is_ok() {
        let retry = engine.observe_invite_response_event(&event)?;
        inner.enqueue_retry_batch(retry)?;
        return Ok(());
    }
    if kind == MESSAGE_EVENT_KIND && parse_message_event(&event).is_ok() {
        let (message, retry) = {
            let message = engine.process_direct_message_event(&event)?;
            let retry =
                engine.retry_pending_protocol(NdrUnixSeconds(event.created_at.as_secs()))?;
            (message, retry)
        };
        if let Some(message) = message {
            inner.enqueue_decrypted_message(message);
        }
        inner.enqueue_retry_batch(retry)?;
    }
    Ok(())
}

fn parse_pubkey(value: &str) -> Result<PublicKey, NdrError> {
    PublicKey::parse(value).map_err(|error| NdrError::InvalidKey(error.to_string()))
}

fn keys_from_private_hex(value: &str) -> Result<Keys, NdrError> {
    let bytes = hex::decode(value)?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| NdrError::InvalidKey("private key must be 32 bytes".to_string()))?;
    let secret = SecretKey::from_slice(&array)?;
    Ok(Keys::new(secret))
}

fn validate_device_pubkey(expected_hex: &str, keys: &Keys) -> Result<(), NdrError> {
    let expected = parse_pubkey(expected_hex)?;
    if expected != keys.public_key() {
        return Err(NdrError::InvalidKey(
            "device public key does not match private key".to_string(),
        ));
    }
    Ok(())
}

fn invite_owner_candidate(
    invite: &iris_chat_protocol::Invite,
    hint: Option<PublicKey>,
) -> PublicKey {
    hint.or(invite.owner_public_key)
        .or_else(|| {
            invite
                .inviter_owner_pubkey
                .and_then(|owner| PublicKey::parse(&owner.to_string()).ok())
        })
        .unwrap_or(invite.inviter)
}

fn invalid_event_error(error: impl std::fmt::Display) -> NdrError {
    NdrError::InvalidEvent(error.to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(keys: &Keys) -> Arc<SessionManagerHandle> {
        manager_with_owner(keys, keys.public_key())
    }

    fn manager_with_owner(keys: &Keys, owner_pubkey: PublicKey) -> Arc<SessionManagerHandle> {
        SessionManagerHandle::new(
            keys.public_key().to_hex(),
            keys.secret_key().to_secret_hex(),
            keys.public_key().to_hex(),
            Some(owner_pubkey.to_hex()),
        )
        .expect("manager")
    }

    fn publish_events(events: Vec<PubSubEvent>, kind: u32) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|event| event.event_json)
            .filter(|event_json| {
                serde_json::from_str::<Event>(event_json)
                    .map(|event| event.kind.as_u16() as u32 == kind)
                    .unwrap_or(false)
            })
            .collect()
    }

    fn invite_events(events: Vec<PubSubEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|event| event.event_json)
            .filter(|event_json| {
                serde_json::from_str::<Event>(event_json)
                    .ok()
                    .and_then(|event| parse_invite_event(&event).ok())
                    .is_some()
            })
            .collect()
    }

    #[test]
    fn oob_invite_handshake_and_direct_message_round_trip() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice = manager(&alice_keys);
        let bob = manager(&bob_keys);

        alice.init().expect("alice init");
        bob.init().expect("bob init");

        let bob_invites = invite_events(bob.drain_events().expect("bob startup drain"));
        let bob_invite = bob_invites.first().expect("bob invite").clone();
        let invite = InviteHandle::from_event_json(bob_invite.clone()).expect("invite handle");
        let compact_url = invite.to_url("https://b".to_string()).expect("invite url");
        let from_url = InviteHandle::from_url(compact_url).expect("url invite");
        assert_eq!(
            from_url.get_inviter_pubkey_hex(),
            bob_keys.public_key().to_hex()
        );

        let accepted = alice
            .accept_invite_from_event_json(bob_invite, None)
            .expect("alice accepts invite");
        assert_eq!(accepted.owner_pubkey_hex, bob_keys.public_key().to_hex());
        assert!(accepted.created_new_session);
        assert!(alice
            .get_active_session_state(bob_keys.public_key().to_hex())
            .expect("alice state")
            .is_some());

        let alice_accept_events = alice.drain_events().expect("alice accept drain");
        for event_json in publish_events(alice_accept_events.clone(), INVITE_RESPONSE_KIND) {
            bob.process_event(event_json)
                .expect("bob processes invite response");
        }
        for event_json in publish_events(alice_accept_events, MESSAGE_EVENT_KIND) {
            bob.process_event(event_json)
                .expect("bob processes bootstrap");
        }
        assert!(bob
            .get_active_session_state(alice_keys.public_key().to_hex())
            .expect("bob state")
            .is_some());
        let _ = bob.drain_events().expect("bob bootstrap drain");

        alice
            .send_text(bob_keys.public_key().to_hex(), "hello".to_string(), None)
            .expect("send text");
        for event_json in publish_events(
            alice.drain_events().expect("alice send drain"),
            MESSAGE_EVENT_KIND,
        ) {
            bob.process_event(event_json)
                .expect("bob processes message");
        }
        let decrypted = bob
            .drain_events()
            .expect("bob drain")
            .into_iter()
            .find(|event| event.kind == "decrypted_message")
            .expect("decrypted message");
        assert!(decrypted.content.expect("content").contains("hello"));
    }

    #[test]
    fn processing_app_keys_snapshot_tracks_peer_owner() {
        let alice_keys = Keys::generate();
        let bob_owner_keys = Keys::generate();
        let bob_device_keys = Keys::generate();
        let alice = manager(&alice_keys);

        alice.init().expect("alice init");
        let _ = alice.drain_events().expect("alice startup drain");
        let roster_created_at = now_secs().saturating_add(1);
        let bob_app_keys = AppKeys::new(vec![DeviceEntry::new(
            bob_device_keys.public_key(),
            roster_created_at,
        )]);
        let bob_app_keys_event = bob_app_keys
            .get_event_at(bob_owner_keys.public_key(), roster_created_at)
            .sign_with_keys(&bob_owner_keys)
            .expect("signed app keys event");

        alice
            .process_event(serde_json::to_string(&bob_app_keys_event).expect("app keys json"))
            .expect("alice processes bob AppKeys");
        let after_roster = alice.drain_events().expect("alice roster drain");
        assert!(publish_events(after_roster, MESSAGE_EVENT_KIND).is_empty());
        assert!(
            alice
                .known_peer_owner_pubkeys()
                .contains(&bob_owner_keys.public_key().to_hex()),
            "shared AppKeys snapshots should surface the peer owner"
        );
    }

    #[test]
    fn key_helpers_round_trip() {
        let pair = generate_keypair();
        assert_eq!(
            derive_public_key(pair.private_key_hex).expect("derive"),
            pair.public_key_hex
        );
    }
}
