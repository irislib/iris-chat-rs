use super::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

// Layout (under `data_dir/`):
//
//     core/
//       meta.json          version, active_chat_id, next_message_id,
//                          authorization_state, seen_event_ids
//       preferences.json   `PersistedPreferences`
//       profiles.json      owner_profiles map
//       app_keys.json      `Vec<KnownAppKeys>`
//       groups.json        `Vec<GroupData>`
//       chat_ttls.json     chat_message_ttl_seconds map
//       threads/<id>.json  one file per chat thread
//
// Persisting on every mutation is split into per-slice files (and
// especially per-chat thread files), so a relay event for one chat only
// rewrites that chat's small file. Each writer runs on
// `runtime.spawn_blocking`, off the core message-handling thread.

const CORE_DIR: &str = "core";
const META_FILE: &str = "meta.json";
const SEEN_EVENTS_FILE: &str = "seen_events.json";
const PREFERENCES_FILE: &str = "preferences.json";
const PROFILES_FILE: &str = "profiles.json";
const APP_KEYS_FILE: &str = "app_keys.json";
const GROUPS_FILE: &str = "groups.json";
const CHAT_TTLS_FILE: &str = "chat_ttls.json";
const THREADS_DIR: &str = "threads";

#[derive(Default)]
pub(super) struct PersistenceCache {
    /// Hash of last-written bytes per slice file. Used to skip rewriting
    /// files whose content hasn't changed.
    pub(super) meta: u64,
    pub(super) seen_events: u64,
    pub(super) preferences: u64,
    pub(super) profiles: u64,
    pub(super) app_keys: u64,
    pub(super) groups: u64,
    pub(super) chat_ttls: u64,
    pub(super) threads: HashMap<String, u64>,
}

impl PersistenceCache {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedMeta {
    version: u32,
    #[serde(default)]
    active_chat_id: Option<String>,
    next_message_id: u64,
    #[serde(default)]
    authorization_state: Option<PersistedAuthorizationState>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedSeenEvents {
    #[serde(default)]
    seen_event_ids: Vec<String>,
}

impl AppCore {
    pub(super) fn ndr_storage_dir(&self, owner: PublicKey, device: PublicKey) -> PathBuf {
        self.data_dir
            .join("ndr_runtime")
            .join(owner.to_hex())
            .join(device.to_hex())
    }

    pub(super) fn debug_snapshot_path(&self) -> PathBuf {
        self.data_dir.join(DEBUG_SNAPSHOT_FILENAME)
    }

    fn core_dir(&self) -> PathBuf {
        self.data_dir.join(CORE_DIR)
    }

    fn meta_path(&self) -> PathBuf {
        self.core_dir().join(META_FILE)
    }

    fn seen_events_path(&self) -> PathBuf {
        self.core_dir().join(SEEN_EVENTS_FILE)
    }

    fn preferences_path(&self) -> PathBuf {
        self.core_dir().join(PREFERENCES_FILE)
    }

    fn profiles_path(&self) -> PathBuf {
        self.core_dir().join(PROFILES_FILE)
    }

    fn app_keys_path(&self) -> PathBuf {
        self.core_dir().join(APP_KEYS_FILE)
    }

    fn groups_path(&self) -> PathBuf {
        self.core_dir().join(GROUPS_FILE)
    }

    fn chat_ttls_path(&self) -> PathBuf {
        self.core_dir().join(CHAT_TTLS_FILE)
    }

    fn threads_dir(&self) -> PathBuf {
        self.core_dir().join(THREADS_DIR)
    }

    fn thread_path(&self, chat_id: &str) -> PathBuf {
        self.threads_dir()
            .join(format!("{}.json", thread_filename(chat_id)))
    }

    pub(super) fn load_persisted(&self) -> anyhow::Result<Option<PersistedState>> {
        if !self.meta_path().exists() {
            return Ok(None);
        }
        let meta_bytes = fs::read(self.meta_path())?;
        let meta: PersistedMeta = serde_json::from_slice(&meta_bytes)?;
        if meta.version != PERSISTED_STATE_VERSION {
            return Ok(None);
        }

        let seen_events: PersistedSeenEvents =
            read_optional_json(&self.seen_events_path())?.unwrap_or_default();
        let preferences = read_optional_json(&self.preferences_path())?.unwrap_or_default();
        let owner_profiles = read_optional_json(&self.profiles_path())?.unwrap_or_default();
        let app_keys: Vec<KnownAppKeys> =
            read_optional_json(&self.app_keys_path())?.unwrap_or_default();
        let groups: Vec<GroupData> = read_optional_json(&self.groups_path())?.unwrap_or_default();
        let chat_message_ttl_seconds: BTreeMap<String, u64> =
            read_optional_json(&self.chat_ttls_path())?.unwrap_or_default();

        // Threads: walk the directory; one file per chat. A missing
        // directory is fine (means no threads were ever persisted).
        let mut threads = Vec::new();
        let threads_dir = self.threads_dir();
        if threads_dir.exists() {
            for entry in fs::read_dir(&threads_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                if let Ok(thread) = serde_json::from_slice::<PersistedThread>(&bytes) {
                    threads.push(thread);
                }
            }
        }

        Ok(Some(PersistedState {
            version: meta.version,
            active_chat_id: meta.active_chat_id,
            next_message_id: meta.next_message_id,
            owner_profiles,
            preferences,
            chat_message_ttl_seconds,
            app_keys,
            groups,
            threads,
            seen_event_ids: seen_events.seen_event_ids,
            authorization_state: meta.authorization_state,
        }))
    }

    pub(super) fn persist_best_effort(&mut self) {
        if self.batch_depth > 0 {
            self.batch_dirty_persist = true;
            return;
        }
        self.persist_best_effort_inner();
    }

    pub(super) fn persist_best_effort_inner(&mut self) {
        if self.logged_in.is_none() {
            return;
        }

        // Build each slice's serialized JSON on the calling thread (a
        // few hundred kilobytes of clones at most), but compare against
        // a hash of the last-written bytes and skip the disk write
        // entirely when the slice is unchanged. The actual fs::write
        // for changed slices is dispatched to the tokio blocking pool
        // so the core thread never waits on the disk.
        let core_dir = self.core_dir();
        let threads_dir = self.threads_dir();
        let mut writes: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let mut deletes: Vec<PathBuf> = Vec::new();

        // ---- meta.json -----------------------------------------------------
        // Tiny file: just version + active_chat_id + next_message_id +
        // auth state. Rewrites whenever you tap a new chat — but it's
        // ~150 bytes so the cost is negligible.
        let auth_state = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.authorization_state.into());
        let meta = PersistedMeta {
            version: PERSISTED_STATE_VERSION,
            active_chat_id: self.active_chat_id.clone(),
            next_message_id: self.next_message_id,
            authorization_state: auth_state,
        };
        if let Some(bytes) = serialise_if_changed(&meta, &mut self.persistence_cache.meta) {
            writes.push((self.meta_path(), bytes));
        }

        // ---- seen_events.json ---------------------------------------------
        // Held separately because (a) it's the largest field on disk
        // (~70 bytes × MAX_SEEN_EVENT_IDS) and (b) it only changes when
        // a fresh relay event arrives, never on UI-triggered actions
        // like opening a chat. Splitting it keeps OpenChat's persist
        // tick at <1 KB instead of ~60 KB.
        let seen_events = PersistedSeenEvents {
            seen_event_ids: self.seen_event_order.iter().cloned().collect(),
        };
        if let Some(bytes) =
            serialise_if_changed(&seen_events, &mut self.persistence_cache.seen_events)
        {
            writes.push((self.seen_events_path(), bytes));
        }

        // ---- preferences.json ---------------------------------------------
        let preferences = PersistedPreferences {
            send_typing_indicators: self.preferences.send_typing_indicators,
            send_read_receipts: self.preferences.send_read_receipts,
            desktop_notifications_enabled: self.preferences.desktop_notifications_enabled,
            startup_at_login_enabled: self.preferences.startup_at_login_enabled,
            nostr_relay_urls: self.preferences.nostr_relay_urls.clone(),
            image_proxy_enabled: self.preferences.image_proxy_enabled,
            image_proxy_url: self.preferences.image_proxy_url.clone(),
            image_proxy_key_hex: self.preferences.image_proxy_key_hex.clone(),
            image_proxy_salt_hex: self.preferences.image_proxy_salt_hex.clone(),
            mobile_push_server_url: self.preferences.mobile_push_server_url.clone(),
        };
        if let Some(bytes) =
            serialise_if_changed(&preferences, &mut self.persistence_cache.preferences)
        {
            writes.push((self.preferences_path(), bytes));
        }

        // ---- profiles.json -------------------------------------------------
        if let Some(bytes) =
            serialise_if_changed(&self.owner_profiles, &mut self.persistence_cache.profiles)
        {
            writes.push((self.profiles_path(), bytes));
        }

        // ---- app_keys.json -------------------------------------------------
        let app_keys: Vec<KnownAppKeys> = self.app_keys.values().cloned().collect();
        if let Some(bytes) = serialise_if_changed(&app_keys, &mut self.persistence_cache.app_keys) {
            writes.push((self.app_keys_path(), bytes));
        }

        // ---- groups.json ---------------------------------------------------
        let groups: Vec<GroupData> = self.groups.values().cloned().collect();
        if let Some(bytes) = serialise_if_changed(&groups, &mut self.persistence_cache.groups) {
            writes.push((self.groups_path(), bytes));
        }

        // ---- chat_ttls.json ------------------------------------------------
        if let Some(bytes) = serialise_if_changed(
            &self.chat_message_ttl_seconds,
            &mut self.persistence_cache.chat_ttls,
        ) {
            writes.push((self.chat_ttls_path(), bytes));
        }

        // ---- threads/<chat_id>.json ---------------------------------------
        // Walk current threads, write the ones whose bytes changed, and
        // queue file deletes for any cached threads that are no longer
        // present (chat removed).
        let mut current_chat_ids: HashSet<String> = HashSet::new();
        for thread in self.threads.values() {
            current_chat_ids.insert(thread.chat_id.clone());
            let persisted = persisted_thread_from_record(thread);
            let entry = self
                .persistence_cache
                .threads
                .entry(thread.chat_id.clone())
                .or_insert(0);
            if let Some(bytes) = serialise_if_changed(&persisted, entry) {
                writes.push((self.thread_path(&thread.chat_id), bytes));
            }
        }
        let stale_thread_ids: Vec<String> = self
            .persistence_cache
            .threads
            .keys()
            .filter(|chat_id| !current_chat_ids.contains(chat_id.as_str()))
            .cloned()
            .collect();
        for chat_id in &stale_thread_ids {
            self.persistence_cache.threads.remove(chat_id);
            deletes.push(self.thread_path(chat_id));
        }

        if writes.is_empty() && deletes.is_empty() {
            self.persist_debug_snapshot_best_effort();
            return;
        }

        self.runtime.spawn_blocking(move || {
            let _ = fs::create_dir_all(&core_dir);
            let _ = fs::create_dir_all(&threads_dir);
            for (path, bytes) in writes {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(path, bytes);
            }
            for path in deletes {
                let _ = fs::remove_file(path);
            }
        });

        self.persist_debug_snapshot_best_effort();
    }

    pub(super) fn clear_persistence_best_effort(&mut self) {
        let core_dir = self.core_dir();
        let debug_path = self.debug_snapshot_path();
        if debug_path.exists() {
            let _ = fs::remove_file(debug_path);
        }
        if core_dir.exists() {
            let _ = fs::remove_dir_all(core_dir);
        }
        self.persistence_cache.clear();
    }

    pub(super) fn persist_debug_snapshot_best_effort(&self) {
        if self.logged_in.is_none() {
            return;
        }
        let snapshot = self.build_runtime_debug_snapshot();
        let path = self.debug_snapshot_path();
        let data_dir = self.data_dir.clone();
        self.runtime.spawn_blocking(move || {
            if let Ok(bytes) = serde_json::to_vec_pretty(&snapshot) {
                let _ = fs::create_dir_all(&data_dir);
                let _ = fs::write(path, bytes);
            }
        });
    }
}

fn persisted_thread_from_record(thread: &ThreadRecord) -> PersistedThread {
    PersistedThread {
        chat_id: thread.chat_id.clone(),
        unread_count: thread.unread_count,
        updated_at_secs: thread.updated_at_secs,
        messages: thread
            .messages
            .iter()
            .map(|message| PersistedMessage {
                id: message.id.clone(),
                chat_id: message.chat_id.clone(),
                kind: message.kind.clone(),
                author: message.author.clone(),
                body: message.body.clone(),
                attachments: message.attachments.clone(),
                reactions: message.reactions.clone(),
                reactors: message.reactors.clone(),
                is_outgoing: message.is_outgoing,
                created_at_secs: message.created_at_secs,
                expires_at_secs: message.expires_at_secs,
                delivery: (&message.delivery).into(),
            })
            .collect(),
    }
}

/// Serialise `value` with `serde_json::to_vec_pretty` and return the
/// bytes only when the resulting hash differs from `cache`. Updates
/// `cache` in place. Returns `None` when nothing changed (skip the
/// write entirely).
fn serialise_if_changed<T: Serialize>(value: &T, cache: &mut u64) -> Option<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(value).ok()?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let hash = hasher.finish();
    if hash == *cache {
        return None;
    }
    *cache = hash;
    Some(bytes)
}

fn read_optional_json<T: serde::de::DeserializeOwned + Default>(
    path: &PathBuf,
) -> anyhow::Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// Map a chat_id (hex pubkey or `g_<hex>`) to a filename-safe form.
/// We just replace any character that isn't `[A-Za-z0-9._-]` with `_`.
/// Uniqueness is preserved because chat_ids are themselves drawn from
/// a near-alphanumeric vocabulary.
fn thread_filename(chat_id: &str) -> String {
    let mut out = String::with_capacity(chat_id.len());
    for ch in chat_id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_filename_preserves_hex_and_neutralises_separators() {
        assert_eq!(
            thread_filename("a4d2d3bb0827f6c3aef2bbbf0d94dfbc"),
            "a4d2d3bb0827f6c3aef2bbbf0d94dfbc"
        );
        assert_eq!(thread_filename("g_a4d2/d3bb"), "g_a4d2_d3bb");
    }
}
