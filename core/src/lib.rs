mod actions;
mod core;
pub mod image_proxy;
pub mod local_relay;
pub mod perflog;
mod qr;
mod state;
mod updates;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use flume::{Receiver, Sender};

pub use actions::AppAction;
pub use qr::*;
pub use state::*;
pub use updates::*;

use crate::core::AppCore;

uniffi::setup_scaffolding!();

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    core_tx: Sender<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String, _keychain_group: String, _app_version: String) -> Arc<Self> {
        let (update_tx, update_rx) = flume::unbounded();
        let (core_tx, core_rx) = flume::unbounded();
        let shared_state = Arc::new(RwLock::new(AppState::empty()));

        let core_tx_for_thread = core_tx.clone();
        let shared_for_thread = shared_state.clone();
        thread::spawn(move || {
            let mut core = AppCore::new(update_tx, core_tx_for_thread, data_dir, shared_for_thread);
            // Drain whatever is already queued and process it as one batch so
            // a flurry of relay events + user actions produces a single UI
            // update instead of N. Without this, tapping a chat while a
            // relay backlog drains can take seconds because OpenChat sits
            // behind every queued event and the UI recomposes between each.
            while let Ok(first) = core_rx.recv() {
                let mut batch = Vec::with_capacity(8);
                batch.push(first);
                while let Ok(next) = core_rx.try_recv() {
                    batch.push(next);
                }
                let batch_size = batch.len();
                let t0 = crate::perflog::now_ms();
                crate::perflog!("core.batch.start size={batch_size}");
                if !core.handle_messages(batch) {
                    break;
                }
                crate::perflog!(
                    "core.batch.end size={batch_size} elapsed_ms={}",
                    crate::perflog::now_ms().saturating_sub(t0)
                );
            }
        });

        Arc::new(Self {
            core_tx,
            update_rx,
            listening: AtomicBool::new(false),
            shared_state,
        })
    }

    pub fn state(&self) -> AppState {
        match self.shared_state.read() {
            Ok(slot) => slot.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    pub fn dispatch(&self, action: AppAction) {
        crate::perflog!("ffi.dispatch action={:?}", std::mem::discriminant(&action));
        let _ = self.core_tx.send(CoreMsg::Action(action));
    }

    pub fn export_support_bundle_json(&self) -> String {
        let (reply_tx, reply_rx) = flume::bounded(1);
        if self
            .core_tx
            .send(CoreMsg::ExportSupportBundle(reply_tx))
            .is_err()
        {
            return "{}".to_string();
        }
        reply_rx.recv().unwrap_or_else(|_| "{}".to_string())
    }

    pub fn shutdown(&self) {
        let (reply_tx, reply_rx) = flume::bounded(1);
        if self
            .core_tx
            .send(CoreMsg::Shutdown(Some(reply_tx)))
            .is_err()
        {
            return;
        }
        let _ = reply_rx.recv();
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let update_rx = self.update_rx.clone();
        thread::spawn(move || {
            // Drain queued updates and deliver the latest FullState only.
            // The shell side already discards FullStates with stale `rev`,
            // but the JNI marshal of an AppState is itself ~20-30 ms and
            // each push triggers a full Compose recomposition (~400 ms on
            // Android debug). When the core emits a tight burst of 3-4
            // updates (OpenChat → SyncComplete → FetchCatchUpEvents → …)
            // the UI keeps re-rendering for seconds even though only the
            // final state mattered.
            //
            // PersistAccountBundle is a side-effect (key persistence), not
            // a UI update, so we never collapse those — every one must run.
            while let Ok(first) = update_rx.recv() {
                let mut latest_full_state: Option<AppUpdate> = None;
                let mut sidecar: Vec<AppUpdate> = Vec::new();
                let process = |update: AppUpdate,
                                   latest: &mut Option<AppUpdate>,
                                   side: &mut Vec<AppUpdate>| match update {
                    full @ AppUpdate::FullState(_) => *latest = Some(full),
                    other => side.push(other),
                };
                process(first, &mut latest_full_state, &mut sidecar);
                while let Ok(next) = update_rx.try_recv() {
                    process(next, &mut latest_full_state, &mut sidecar);
                }
                for update in sidecar.into_iter().chain(latest_full_state) {
                    let kind = match &update {
                        AppUpdate::FullState(_) => "FullState",
                        AppUpdate::PersistAccountBundle { .. } => "PersistAccountBundle",
                    };
                    let t0 = crate::perflog::now_ms();
                    crate::perflog!("reconcile.start kind={kind}");
                    reconciler.reconcile(update);
                    crate::perflog!(
                        "reconcile.end kind={kind} elapsed_ms={}",
                        crate::perflog::now_ms().saturating_sub(t0)
                    );
                }
            }
        });
    }
}

impl Drop for FfiApp {
    fn drop(&mut self) {
        let _ = self.core_tx.send(CoreMsg::Shutdown(None));
    }
}

#[uniffi::export]
pub fn normalize_peer_input(input: String) -> String {
    crate::core::normalize_peer_input_for_display(&input)
}

#[uniffi::export]
pub fn is_valid_peer_input(input: String) -> bool {
    crate::core::parse_peer_input(&input).is_ok()
}

/// Convert any pubkey-shaped input (hex, npub, nprofile, …) to its
/// canonical lowercase-hex form. The empty string is returned when the
/// input can't be parsed as a public key — callers expecting hex
/// downstream can short-circuit on that.
#[uniffi::export]
pub fn peer_input_to_hex(input: String) -> String {
    let normalized = crate::core::normalize_peer_input_for_display(&input);
    match nostr::PublicKey::parse(&normalized) {
        Ok(pubkey) => pubkey.to_hex(),
        Err(_) => String::new(),
    }
}

/// Convert any pubkey-shaped input (hex, npub, nprofile, …) to its npub form.
/// Returns the original string when it can't be parsed as a public key.
#[uniffi::export]
pub fn peer_input_to_npub(input: String) -> String {
    use nostr::nips::nip19::ToBech32;
    let normalized = crate::core::normalize_peer_input_for_display(&input);
    match nostr::PublicKey::parse(&normalized) {
        Ok(pubkey) => pubkey.to_bech32().unwrap_or(normalized),
        Err(_) => normalized,
    }
}

#[uniffi::export]
pub fn build_summary() -> String {
    crate::core::build_summary()
}

#[uniffi::export]
pub fn relay_set_id() -> String {
    crate::core::relay_set_id().to_string()
}

#[uniffi::export]
pub fn proxied_image_url(
    original_src: String,
    preferences: PreferencesSnapshot,
    width: Option<u32>,
    height: Option<u32>,
    square: bool,
) -> String {
    image_proxy::proxied_image_url(&original_src, &preferences, width, height, square)
}

#[uniffi::export]
pub fn is_trusted_test_build() -> bool {
    crate::core::trusted_test_build_flag()
}

#[uniffi::export]
pub fn resolve_mobile_push_notification_payload(
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    crate::core::resolve_mobile_push_notification(raw_payload_json)
}

/// Decrypt a notification payload against the persisted double-ratchet
/// state under `data_dir`. Use from the FCM service (Android) or
/// Notification Service Extension (iOS) where there's no live `FfiApp`.
/// Falls back to the generic resolver when keys, payload, or storage
/// are unavailable so the user still gets *some* notification.
#[uniffi::export]
pub fn decrypt_mobile_push_notification_payload(
    data_dir: String,
    owner_pubkey_hex: String,
    device_nsec: String,
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    crate::core::decrypt_mobile_push_notification(
        data_dir,
        owner_pubkey_hex,
        device_nsec,
        raw_payload_json,
    )
}

#[uniffi::export]
pub fn resolve_mobile_push_subscription_server_url(
    platform_key: String,
    is_release: bool,
    override_url: Option<String>,
) -> String {
    crate::core::resolve_mobile_push_server_url(platform_key, is_release, override_url)
}

#[uniffi::export]
pub fn mobile_push_subscription_id_key(platform_key: String) -> String {
    crate::core::mobile_push_stored_subscription_id_key(platform_key)
}

#[uniffi::export]
pub fn build_mobile_push_list_subscriptions_request(
    owner_nsec: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    crate::core::build_mobile_push_list_subscriptions_request(
        owner_nsec,
        platform_key,
        is_release,
        server_url_override,
    )
}

#[uniffi::export]
pub fn build_mobile_push_create_subscription_request(
    owner_nsec: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    crate::core::build_mobile_push_create_subscription_request(
        owner_nsec,
        platform_key,
        push_token,
        apns_topic,
        message_author_pubkeys,
        is_release,
        server_url_override,
    )
}

#[uniffi::export]
pub fn build_mobile_push_update_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    crate::core::build_mobile_push_update_subscription_request(
        owner_nsec,
        subscription_id,
        platform_key,
        push_token,
        apns_topic,
        message_author_pubkeys,
        is_release,
        server_url_override,
    )
}

#[uniffi::export]
pub fn build_mobile_push_delete_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    crate::core::build_mobile_push_delete_subscription_request(
        owner_nsec,
        subscription_id,
        platform_key,
        is_release,
        server_url_override,
    )
}
