use super::*;

impl AppCore {
    pub(super) fn build_runtime_debug_snapshot(&self) -> RuntimeDebugSnapshot {
        let current_protocol_plan =
            self.compute_protocol_subscription_plan()
                .map(|plan| RuntimeProtocolPlanDebug {
                    runtime_subscriptions: plan.runtime_subscriptions,
                });
        let tracked_owner_hexes = sorted_hexes(self.tracked_peer_owner_hexes());
        let current_chat_list = self.threads.keys().cloned().collect::<Vec<_>>();
        let (local_owner_pubkey_hex, local_device_pubkey_hex, authorization_state) =
            if let Some(logged_in) = self.logged_in.as_ref() {
                (
                    Some(logged_in.owner_pubkey.to_hex()),
                    Some(local_device_from_keys(&logged_in.device_keys).to_hex()),
                    Some(format!("{:?}", logged_in.authorization_state)),
                )
            } else {
                (None, None, None)
            };
        let known_users = self
            .app_keys
            .values()
            .map(|known| RuntimeKnownUserDebug {
                owner_pubkey_hex: known.owner_pubkey_hex.clone(),
                has_roster: true,
                roster_device_count: known.devices.len(),
                device_count: known.devices.len(),
                authorized_device_count: known.devices.len(),
                active_session_device_count: 0,
                inactive_session_count: 0,
            })
            .collect::<Vec<_>>();

        RuntimeDebugSnapshot {
            generated_at_secs: unix_now().get(),
            local_owner_pubkey_hex,
            local_device_pubkey_hex,
            authorization_state,
            active_chat_id: self.active_chat_id.clone(),
            current_protocol_plan,
            tracked_owner_hexes,
            known_users,
            recent_handshake_peers: self
                .recent_handshake_peers
                .values()
                .map(|peer| RuntimeRecentHandshakeDebug {
                    owner_hex: peer.owner_hex.clone(),
                    device_hex: peer.device_hex.clone(),
                    observed_at_secs: peer.observed_at_secs,
                })
                .collect(),
            event_counts: self.debug_event_counters.clone(),
            recent_log: self.debug_log.iter().cloned().collect(),
            toast: self.state.toast.clone(),
            current_chat_list,
        }
    }

    pub(super) fn export_support_bundle_json(&self) -> String {
        serde_json::to_string_pretty(&self.build_support_bundle())
            .unwrap_or_else(|_| "{}".to_string())
    }

    pub(super) fn build_support_bundle(&self) -> SupportBundle {
        let runtime = self.build_runtime_debug_snapshot();
        let current_screen = self
            .screen_stack
            .last()
            .cloned()
            .unwrap_or_else(|| self.state.router.default_screen.clone());
        let direct_chat_count = self
            .threads
            .keys()
            .filter(|chat_id| !is_group_chat_id(chat_id))
            .count();
        let group_chat_count = self
            .threads
            .keys()
            .filter(|chat_id| is_group_chat_id(chat_id))
            .count();
        let unread_chat_count = self
            .threads
            .values()
            .filter(|thread| thread.unread_count > 0)
            .count();

        SupportBundle {
            generated_at_secs: unix_now().get(),
            build: SupportBuildMetadata {
                app_version: APP_VERSION.to_string(),
                build_channel: BUILD_CHANNEL.to_string(),
                git_sha: BUILD_GIT_SHA.to_string(),
                build_timestamp_utc: BUILD_TIMESTAMP_UTC.to_string(),
                relay_set_id: RELAY_SET_ID.to_string(),
                trusted_test_build: trusted_test_build(),
            },
            relay_urls: self.preferences.nostr_relay_urls.clone(),
            authorization_state: runtime.authorization_state,
            active_chat_id: runtime.active_chat_id,
            current_screen: format!("{current_screen:?}"),
            chat_count: self.threads.len(),
            direct_chat_count,
            group_chat_count,
            unread_chat_count,
            protocol: runtime.current_protocol_plan,
            tracked_owner_hexes: runtime.tracked_owner_hexes,
            known_users: runtime.known_users,
            recent_handshake_peers: runtime.recent_handshake_peers,
            event_counts: runtime.event_counts,
            recent_log: runtime.recent_log,
            current_chat_list: runtime.current_chat_list,
            latest_toast: runtime.toast,
        }
    }

    pub(super) fn push_debug_log(&mut self, category: &str, detail: impl Into<String>) {
        self.debug_log.push_back(DebugLogEntry {
            timestamp_secs: unix_now().get(),
            category: category.to_string(),
            detail: detail.into(),
        });
        while self.debug_log.len() > MAX_DEBUG_LOG_ENTRIES {
            self.debug_log.pop_front();
        }
    }
}
