use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iris_chat_core::{AppAction, AppReconciler, AppState, AppUpdate, FfiApp, OutgoingAttachment};

use crate::secure_storage::{FileSecretStore, SecretStore, StoredAccountBundle};

pub struct AppManager {
    ffi: Arc<FfiApp>,
    update_rx: async_channel::Receiver<AppUpdate>,
    secret_store: Arc<dyn SecretStore>,
    data_dir: PathBuf,
    staged_attachments: RefCell<HashMap<String, Vec<OutgoingAttachment>>>,
    last_focused_chat_id: RefCell<Option<String>>,
}

struct Reconciler {
    tx: async_channel::Sender<AppUpdate>,
    secret_store: Arc<dyn SecretStore>,
}

impl AppReconciler for Reconciler {
    fn reconcile(&self, update: AppUpdate) {
        if let AppUpdate::PersistAccountBundle {
            owner_nsec,
            owner_pubkey_hex,
            device_nsec,
            ..
        } = &update
        {
            self.secret_store.save(&StoredAccountBundle {
                owner_nsec: owner_nsec.clone(),
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                device_nsec: device_nsec.clone(),
            });
        }
        let _ = self.tx.send_blocking(update);
    }
}

impl AppManager {
    pub fn new() -> Self {
        let data_dir = ensure_dir(xdg_data_home().join("iris-chat"));
        let secrets_dir = ensure_dir(xdg_config_home().join("iris-chat"));
        let secret_store: Arc<dyn SecretStore> = Arc::new(FileSecretStore::new(&secrets_dir));

        let ffi = FfiApp::new(
            data_dir.to_string_lossy().to_string(),
            String::new(),
            env!("CARGO_PKG_VERSION").to_string(),
        );

        let (tx, rx) = async_channel::unbounded();
        ffi.listen_for_updates(Box::new(Reconciler {
            tx,
            secret_store: secret_store.clone(),
        }));

        if let Some(bundle) = secret_store.load() {
            ffi.dispatch(AppAction::RestoreAccountBundle {
                owner_nsec: bundle.owner_nsec,
                owner_pubkey_hex: bundle.owner_pubkey_hex,
                device_nsec: bundle.device_nsec,
            });
        }

        Self {
            ffi,
            update_rx: rx,
            secret_store,
            data_dir,
            staged_attachments: RefCell::new(HashMap::new()),
            last_focused_chat_id: RefCell::new(None),
        }
    }

    pub fn should_focus_composer(&self, chat_id: &str) -> bool {
        let mut slot = self.last_focused_chat_id.borrow_mut();
        if slot.as_deref() == Some(chat_id) {
            return false;
        }
        *slot = Some(chat_id.to_string());
        true
    }

    pub fn staged_attachments(&self, chat_id: &str) -> Vec<OutgoingAttachment> {
        self.staged_attachments
            .borrow()
            .get(chat_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn stage_attachment(&self, chat_id: &str, attachment: OutgoingAttachment) {
        let mut staged = self.staged_attachments.borrow_mut();
        let entry = staged.entry(chat_id.to_string()).or_default();
        if !entry.iter().any(|a| a.file_path == attachment.file_path) {
            entry.push(attachment);
        }
    }

    pub fn unstage_attachment(&self, chat_id: &str, file_path: &str) {
        if let Some(entry) = self.staged_attachments.borrow_mut().get_mut(chat_id) {
            entry.retain(|a| a.file_path != file_path);
        }
    }

    pub fn take_staged_attachments(&self, chat_id: &str) -> Vec<OutgoingAttachment> {
        self.staged_attachments
            .borrow_mut()
            .remove(chat_id)
            .unwrap_or_default()
    }

    pub fn current_state(&self) -> AppState {
        self.ffi.state()
    }

    pub fn update_rx(&self) -> async_channel::Receiver<AppUpdate> {
        self.update_rx.clone()
    }

    pub fn dispatch(&self, action: AppAction) {
        self.ffi.dispatch(action);
    }

    pub fn export_support_bundle_json(&self) -> String {
        self.ffi.export_support_bundle_json()
    }

    #[allow(dead_code)]
    pub fn logout(&self) {
        self.ffi.dispatch(AppAction::Logout);
        self.secret_store.clear();
        let _ = std::fs::remove_dir_all(&self.data_dir);
        let _ = std::fs::create_dir_all(&self.data_dir);
    }
}

fn xdg_data_home() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".local/share")
}

fn xdg_config_home() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".config")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn ensure_dir(path: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&path);
    path
}
