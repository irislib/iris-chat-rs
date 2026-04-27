use super::*;

impl AppCore {
    pub(super) fn persistence_path(&self) -> PathBuf {
        self.data_dir.join("ndr_demo_core_state.json")
    }

    pub(super) fn ndr_storage_dir(&self, owner: PublicKey, device: PublicKey) -> PathBuf {
        self.data_dir
            .join("ndr_runtime")
            .join(owner.to_hex())
            .join(device.to_hex())
    }

    pub(super) fn debug_snapshot_path(&self) -> PathBuf {
        self.data_dir.join(DEBUG_SNAPSHOT_FILENAME)
    }

    pub(super) fn load_persisted(&self) -> anyhow::Result<Option<PersistedState>> {
        let path = self.persistence_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value.get("version").and_then(serde_json::Value::as_u64)
            != Some(PERSISTED_STATE_VERSION as u64)
        {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(value)?))
    }

    pub(super) fn persist_best_effort(&self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };

        let persisted = PersistedState {
            version: PERSISTED_STATE_VERSION,
            active_chat_id: self.active_chat_id.clone(),
            next_message_id: self.next_message_id,
            owner_profiles: self.owner_profiles.clone(),
            preferences: PersistedPreferences {
                send_typing_indicators: self.preferences.send_typing_indicators,
                send_read_receipts: self.preferences.send_read_receipts,
                desktop_notifications_enabled: self.preferences.desktop_notifications_enabled,
                startup_at_login_enabled: self.preferences.startup_at_login_enabled,
                nostr_relay_urls: self.preferences.nostr_relay_urls.clone(),
                image_proxy_enabled: self.preferences.image_proxy_enabled,
                image_proxy_url: self.preferences.image_proxy_url.clone(),
                image_proxy_key_hex: self.preferences.image_proxy_key_hex.clone(),
                image_proxy_salt_hex: self.preferences.image_proxy_salt_hex.clone(),
            },
            chat_message_ttl_seconds: self.chat_message_ttl_seconds.clone(),
            app_keys: self.app_keys.values().cloned().collect(),
            groups: self.groups.values().cloned().collect(),
            threads: self
                .threads
                .values()
                .map(|thread| PersistedThread {
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
                })
                .collect(),
            seen_event_ids: self.seen_event_order.iter().cloned().collect(),
            authorization_state: Some(logged_in.authorization_state.into()),
        };

        if let Ok(bytes) = serde_json::to_vec_pretty(&persisted) {
            let _ = fs::create_dir_all(&self.data_dir);
            let _ = fs::write(self.persistence_path(), bytes);
        }
        self.persist_debug_snapshot_best_effort();
    }

    pub(super) fn clear_persistence_best_effort(&self) {
        let path = self.persistence_path();
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        let debug_path = self.debug_snapshot_path();
        if debug_path.exists() {
            let _ = fs::remove_file(debug_path);
        }
    }

    pub(super) fn persist_debug_snapshot_best_effort(&self) {
        if self.logged_in.is_none() {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.build_runtime_debug_snapshot()) {
            let _ = fs::create_dir_all(&self.data_dir);
            let _ = fs::write(self.debug_snapshot_path(), bytes);
        }
    }
}
