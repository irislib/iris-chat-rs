use super::*;

impl AppCore {
    pub(super) fn build_runtime_debug_snapshot(&self) -> RuntimeDebugSnapshot {
        let current_protocol_plan =
            self.compute_protocol_subscription_plan()
                .map(|plan| RuntimeProtocolPlanDebug {
                    runtime_subscriptions: plan.runtime_subscriptions,
                    roster_authors: plan.roster_authors,
                    invite_authors: plan.invite_authors,
                    message_authors: plan.message_authors,
                    invite_response_recipient: plan.invite_response_recipient,
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
            .map(|known| {
                let owner_pubkey = PublicKey::parse(&known.owner_pubkey_hex).ok();
                let counts = owner_pubkey
                    .map(|owner| self.peer_debug_session_counts(owner))
                    .unwrap_or_default();
                RuntimeKnownUserDebug {
                    owner_pubkey_hex: known.owner_pubkey_hex.clone(),
                    has_roster: true,
                    roster_device_count: known.devices.len(),
                    device_count: known.devices.len(),
                    authorized_device_count: known.devices.len(),
                    active_session_device_count: counts.active_session_count as usize,
                    inactive_session_count: counts
                        .session_count
                        .saturating_sub(counts.active_session_count)
                        as usize,
                }
            })
            .collect::<Vec<_>>();

        RuntimeDebugSnapshot {
            generated_at_secs: unix_now().get(),
            local_owner_pubkey_hex,
            local_device_pubkey_hex,
            authorization_state,
            active_chat_id: self.active_chat_id.clone(),
            current_protocol_plan,
            protocol_engine: self
                .protocol_engine
                .as_ref()
                .map(ProtocolEngine::debug_snapshot),
            pending_relay_publishes: self
                .pending_relay_publishes
                .values()
                .map(|pending| RuntimePendingRelayPublishDebug {
                    event_id: pending.event_id.clone(),
                    label: pending.label.clone(),
                    inner_event_id: pending.inner_event_id.clone(),
                    target_owner_pubkey_hex: pending.target_owner_pubkey_hex.clone(),
                    target_device_id: pending.target_device_id.clone(),
                    message_id: pending.message_id.clone(),
                    chat_id: pending.chat_id.clone(),
                    attempt_count: pending.attempt_count,
                    last_error: pending.last_error.clone(),
                })
                .collect(),
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

    pub(super) fn build_peer_profile_debug_snapshot(
        &self,
        owner_input: &str,
    ) -> Option<PeerProfileDebugSnapshot> {
        let (owner_pubkey_hex, owner_pubkey) = parse_peer_input(owner_input).ok()?;
        if is_group_chat_id(&owner_pubkey_hex) {
            return None;
        }

        let roster_device_count = self
            .app_keys
            .get(&owner_pubkey_hex)
            .map(|known| known.devices.len() as u64)
            .unwrap_or(0);
        let known_device_count = self
            .protocol_engine
            .as_ref()
            .map(|engine| {
                engine
                    .known_device_identity_pubkeys_for_owner(owner_pubkey)
                    .len() as u64
            })
            .unwrap_or(0);
        let counts = self.peer_debug_session_counts(owner_pubkey);
        let recent_handshakes = self
            .recent_handshake_peers
            .values()
            .filter(|peer| peer.owner_hex == owner_pubkey_hex)
            .collect::<Vec<_>>();
        let recent_handshake_device_count = recent_handshakes.len() as u64;
        let last_handshake_at_secs = recent_handshakes
            .iter()
            .map(|peer| peer.observed_at_secs)
            .max();

        Some(PeerProfileDebugSnapshot {
            owner_pubkey_hex: owner_pubkey_hex.clone(),
            owner_npub: owner_npub_from_owner(owner_pubkey)
                .unwrap_or_else(|| owner_pubkey_hex.clone()),
            roster_device_count,
            known_device_count,
            active_session_count: counts.active_session_count,
            session_count: counts.session_count,
            receiving_session_count: counts.receiving_session_count,
            tracked_sender_count: counts.tracked_sender_count,
            recent_handshake_device_count,
            last_handshake_at_secs,
            tracked_for_messages: self.tracked_peer_owner_hexes().contains(&owner_pubkey_hex),
        })
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
            protocol_engine: runtime.protocol_engine,
            pending_relay_publishes: runtime.pending_relay_publishes,
            tracked_owner_hexes: runtime.tracked_owner_hexes,
            known_users: runtime.known_users,
            recent_handshake_peers: runtime.recent_handshake_peers,
            event_counts: runtime.event_counts,
            recent_log: runtime.recent_log,
            current_chat_list: runtime.current_chat_list,
            latest_toast: runtime.toast,
        }
    }

    fn peer_debug_session_counts(&self, owner_pubkey: PublicKey) -> PeerDebugSessionCounts {
        let Some(protocol_engine) = self.protocol_engine.as_ref() else {
            return PeerDebugSessionCounts::default();
        };

        let sessions = protocol_engine.message_session_debug_snapshots(owner_pubkey);
        let tracked_sender_count = sessions
            .iter()
            .flat_map(|session| session.tracked_sender_pubkeys.iter())
            .map(|sender| sender.to_hex())
            .collect::<HashSet<_>>()
            .len() as u64;
        let active_session_count =
            protocol_engine.active_session_count_for_owner(owner_pubkey) as u64;

        PeerDebugSessionCounts {
            active_session_count,
            session_count: sessions.len() as u64,
            receiving_session_count: sessions
                .iter()
                .filter(|session| session.has_receiving_capability)
                .count() as u64,
            tracked_sender_count,
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

    pub(crate) fn mark_core_panic(&mut self, detail: String) {
        crate::perflog!("core.batch.panic detail={detail}");
        self.push_debug_log("core.panic", detail);
        self.state.toast = Some("Iris needs restart. Copy support bundle in Settings.".to_string());
        self.persist_debug_snapshot_best_effort();
        self.emit_state();
    }
}

#[derive(Default)]
struct PeerDebugSessionCounts {
    active_session_count: u64,
    session_count: u64,
    receiving_session_count: u64,
    tracked_sender_count: u64,
}
