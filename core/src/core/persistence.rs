use super::storage::SaveSnapshot;
use super::*;

// Durable app state lives in `data_dir/core.sqlite3` (managed by
// `core::storage`). The notification-extension preview cache and the
// runtime debug snapshot are still file-backed because they must be
// readable from contexts that don't open the SQLite database (the
// FCM service / iOS Notification Service Extension paths read JSON
// directly).

impl AppCore {
    pub(super) fn debug_snapshot_path(&self) -> PathBuf {
        self.data_dir.join(DEBUG_SNAPSHOT_FILENAME)
    }

    pub(super) fn load_persisted(&mut self) -> anyhow::Result<Option<PersistedState>> {
        self.app_store.delete_expired_messages(unix_now().get())?;
        self.app_store.load_state()
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

        let authorization_state = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.authorization_state.into());
        let snapshot = SaveSnapshot {
            active_chat_id: self.active_chat_id.as_deref(),
            next_message_id: self.next_message_id,
            authorization_state,
            preferences: &self.preferences,
            owner_profiles: &self.owner_profiles,
            chat_message_ttl_seconds: &self.chat_message_ttl_seconds,
            app_keys: &self.app_keys,
            groups: &self.groups,
            threads: &self.threads,
            seen_event_order: &self.seen_event_order,
        };
        match self.app_store.save_state(&snapshot) {
            Ok(()) => {
                self.ack_pending_decrypted_deliveries_after_app_persist();
            }
            Err(error) => {
                self.push_debug_log("storage.save_failed", error.to_string());
            }
        }

        self.persist_debug_snapshot_best_effort();
    }

    pub(super) fn clear_persistence_best_effort(&mut self) {
        if let Err(error) = self.app_store.clear() {
            self.push_debug_log("storage.clear_failed", error.to_string());
        }
        let debug_path = self.debug_snapshot_path();
        if debug_path.exists() {
            let _ = fs::remove_file(debug_path);
        }
    }

    pub(super) fn persist_debug_snapshot_best_effort(&mut self) {
        if self.logged_in.is_none() {
            return;
        }
        // The on-disk snapshot is a test-harness fixture (only read
        // by `core/tests`, iOS `InteropHarnessTests`, Android
        // `RealRelayHarnessTest`). Production never reads it; the
        // user-facing support bundle rebuilds it in-memory at export
        // time via `build_support_bundle()`. So we gate the
        // continuous file write to debug builds + an explicit env
        // override for the rare release-build test lane.
        if !debug_snapshot_file_writes_enabled() {
            return;
        }
        if self.debug_snapshot_write_inflight {
            self.debug_snapshot_write_dirty = true;
            return;
        }
        // Throttle: even in debug builds, every relay event /
        // protocol persist used to fan out into a full SessionManager
        // clone × N known users + a JSON write. Tests poll the file
        // with multi-second budgets, so a 5s floor is invisible to
        // them and stops the loop the release sample caught.
        let now_ms = unix_now_ms();
        if now_ms.saturating_sub(self.debug_snapshot_last_built_at_ms)
            < DEBUG_SNAPSHOT_MIN_INTERVAL_MS
        {
            self.debug_snapshot_write_dirty = true;
            return;
        }
        let snapshot = self.build_runtime_debug_snapshot();
        self.debug_snapshot_build_count = self.debug_snapshot_build_count.saturating_add(1);
        self.debug_snapshot_last_built_at_ms = now_ms;
        let path = self.debug_snapshot_path();
        let data_dir = self.data_dir.clone();

        #[cfg(target_os = "ios")]
        {
            if let Ok(bytes) = serde_json::to_vec_pretty(&snapshot) {
                let _ = fs::create_dir_all(&data_dir);
                let _ = fs::write(path, bytes);
            }
            return;
        }

        #[cfg(not(target_os = "ios"))]
        {
            self.debug_snapshot_write_inflight = true;
            self.debug_snapshot_write_dirty = false;
            self.debug_snapshot_write_generation =
                self.debug_snapshot_write_generation.wrapping_add(1);
            let generation = self.debug_snapshot_write_generation;
            let tx = self.core_sender.clone();
            self.runtime.spawn_blocking(move || {
                if let Ok(bytes) = serde_json::to_vec_pretty(&snapshot) {
                    let _ = fs::create_dir_all(&data_dir);
                    let _ = fs::write(path, bytes);
                }
                let _ = tx.send(CoreMsg::Internal(Box::new(
                    InternalEvent::DebugSnapshotWriteFinished { generation },
                )));
            });
        }
    }

    /// Read by `FfiApp::core_perf_counters` so the release gate can
    /// budget core-internal hot loops, not just FFI surface traffic.
    pub(crate) fn debug_snapshot_build_count(&self) -> u64 {
        self.debug_snapshot_build_count
    }

    #[cfg(not(target_os = "ios"))]
    pub(super) fn handle_debug_snapshot_write_finished(&mut self, generation: u64) {
        if generation != self.debug_snapshot_write_generation {
            return;
        }
        self.debug_snapshot_write_inflight = false;
        if self.debug_snapshot_write_dirty {
            self.debug_snapshot_write_dirty = false;
            self.persist_debug_snapshot_best_effort();
        }
    }
}
