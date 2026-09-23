use super::config::COMPILED_DEVICE_APPROVAL_RELAY_URL;
use super::persistence::apply_persisted_preferences;
use super::*;
mod actions;

pub(super) const CATCH_UP_EVENT_PROCESS_CHUNK_SIZE: usize = 64;
const APP_RUNTIME_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

impl AppCore {
    #[cfg(test)]
    pub fn new(
        update_tx: Sender<AppUpdate>,
        core_sender: Sender<CoreMsg>,
        data_dir: String,
        shared_state: Arc<RwLock<AppState>>,
    ) -> Self {
        Self::try_new(update_tx, core_sender, data_dir, shared_state).expect("start app core")
    }

    #[cfg(test)]
    pub fn try_new(
        update_tx: Sender<AppUpdate>,
        core_sender: Sender<CoreMsg>,
        data_dir: String,
        shared_state: Arc<RwLock<AppState>>,
    ) -> anyhow::Result<Self> {
        Self::try_new_with_priority_sender(
            update_tx,
            core_sender.clone(),
            core_sender,
            data_dir,
            shared_state,
        )
    }

    pub fn try_new_with_priority_sender(
        update_tx: Sender<AppUpdate>,
        core_sender: Sender<CoreMsg>,
        priority_sender: Sender<CoreMsg>,
        data_dir: String,
        shared_state: Arc<RwLock<AppState>>,
    ) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .max_blocking_threads(8)
            // FIPS control-plane turns contain several deliberately bounded
            // async layers. Android's default Tokio worker stack overflows in
            // debug builds while polling a tree announce immediately after a
            // BLE Noise handshake, before the application receipt can run.
            .thread_stack_size(APP_RUNTIME_THREAD_STACK_BYTES)
            .build()?;

        let data_dir = PathBuf::from(data_dir);
        let data_dir_lock = DataDirLock::acquire(&data_dir)?;
        let mut app_store = AppStore::new(open_database(&data_dir)?);

        let mut state = AppState::empty();
        if let Some(persisted_preferences) = app_store.load_preferences_snapshot()? {
            apply_persisted_preferences(&mut state.preferences, &persisted_preferences);
        }
        match shared_state.write() {
            Ok(mut slot) => *slot = state.clone(),
            Err(poison) => *poison.into_inner() = state.clone(),
        }

        Ok(Self {
            update_tx,
            core_sender,
            priority_sender,
            shared_state,
            runtime,
            data_dir,
            state: state.clone(),
            logged_in: None,
            protocol_engine: None,
            pending_linked_device: None,
            pending_signer_login: None,
            pending_remote_signer: None,
            device_approval_relay_urls: relay_urls_from_strings(&[
                COMPILED_DEVICE_APPROVAL_RELAY_URL.to_string(),
            ]),
            private_chat_invites: BTreeMap::new(),
            pending_private_invite_responses: BTreeMap::new(),
            pending_private_invite_cleanup_retry: false,
            pending_outgoing_invite_acceptance: None,
            threads: BTreeMap::new(),
            chat_deletions: BTreeMap::new(),
            chat_read_states: BTreeMap::new(),
            active_chat_id: None,
            screen_stack: Vec::new(),
            next_message_id: 1,
            owner_profiles: BTreeMap::new(),
            profile_metadata_fetch_inflight: HashSet::new(),
            app_keys: BTreeMap::new(),
            direct_chat_capability_runtime: DirectChatCapabilityRuntime::default(),
            user_discovery: UserDiscoveryCache::default(),
            user_discovery_runtime: UserDiscoveryRuntime::default(),
            profile_search_runtime: ProfileSearchRuntime::default(),
            user_discovery_revision: 0,
            user_discovery_syncing: false,
            groups: BTreeMap::new(),
            group_pictures: BTreeMap::new(),
            typing_indicators: BTreeMap::new(),
            typing_floor_secs: BTreeMap::new(),
            chat_message_ttl_seconds: BTreeMap::new(),
            preferences: state.preferences.clone(),
            recent_handshake_peers: BTreeMap::new(),
            seen_event_ids: HashSet::new(),
            seen_event_order: VecDeque::new(),
            device_invite_poll_token: 0,
            pending_device_link_poll_token: 0,
            message_expiry_token: 0,
            protocol_reconnect_token: 0,
            protocol_liveness_token: 0,
            defer_owner_app_keys_publish: false,
            owner_registration_lookup_generation: None,
            current_device_labels: None,
            protocol_subscription_runtime: ProtocolSubscriptionRuntime::default(),
            relay_transport_runtime: RelayTransportRuntime::default(),
            relay_status_watch_urls: HashSet::new(),
            relay_status_watch_generation: 0,
            relay_status_by_url: BTreeMap::new(),
            relay_connected_count: 0,
            all_relays_offline_since_secs: None,
            device_sync: None,
            calls: calls::CallRuntime::default(),
            pending_host_ble: None,
            host_ble_attached: false,
            fips_nearby_links: Vec::new(),
            fips_connection_generation: 0,
            pending_relay_publishes: BTreeMap::new(),
            pending_relay_publish_inflight: HashSet::new(),
            pending_decrypted_delivery_acks: HashSet::new(),
            event_transport_channels: BTreeMap::new(),
            pending_mobile_push_events: VecDeque::new(),
            debug_log: VecDeque::new(),
            debug_event_counters: DebugEventCounters::default(),
            debug_snapshot_write_generation: 0,
            debug_snapshot_write_inflight: false,
            debug_snapshot_write_dirty: false,
            debug_snapshot_last_built_at_ms: 0,
            debug_snapshot_build_count: 0,
            batch_depth: 0,
            batch_dirty_state: false,
            batch_dirty_persist: false,
            pending_outgoing_receipts: BTreeMap::new(),
            pending_delivered_receipts: BTreeMap::new(),
            pending_delivered_receipt_flush_due_at: None,
            pending_delivered_receipt_token: 0,
            last_emitted_state: None,
            app_store,
            _data_dir_lock: data_dir_lock,
            cached_mobile_push: MobilePushSyncSnapshot::default(),
            // First rebuild populates the cache.
            mobile_push_dirty: true,
            suspended: false,
        })
    }

    /// Clone of the SQLite connection handle used by the core thread.
    /// Search runs on the FFI thread directly to avoid queueing behind
    /// `OpenChat`/relay-event batches; the per-connection mutex inside
    /// `SharedConnection` keeps that safe against concurrent writes.
    pub(crate) fn shared_db(&self) -> super::storage::SharedConnection {
        self.app_store.shared()
    }

    pub fn handle_message(&mut self, msg: CoreMsg) -> bool {
        let t0 = crate::perflog::now_ms();
        let label: &'static str = match &msg {
            CoreMsg::Action(action) => match action {
                AppAction::OpenChat { .. } => "OpenChat",
                AppAction::SendMessage { .. } => "SendMessage",
                AppAction::PushScreen { .. } => "PushScreen",
                AppAction::NavigateBack => "NavigateBack",
                AppAction::UpdateScreenStack { .. } => "UpdateScreenStack",
                AppAction::AppForegrounded => "AppForegrounded",
                AppAction::MarkMessagesSeen { .. } => "MarkMessagesSeen",
                _ => "Action.other",
            },
            CoreMsg::Internal(event) => match event.as_ref() {
                InternalEvent::RemoteSignerProgress { .. }
                | InternalEvent::RemoteSignerConnected { .. }
                | InternalEvent::RemoteSignerSigned { .. }
                | InternalEvent::RemoteSignerFailed { .. } => "RemoteSigner",
                InternalEvent::CallPacket { .. } => "CallPacket",
                InternalEvent::CallTick { .. } => "CallTick",
                InternalEvent::CallRecoveryTick { .. } => "CallRecoveryTick",
                InternalEvent::SignerLoginFetched { .. } => "SignerLoginFetched",
                InternalEvent::SignerLoginPublished { .. } => "SignerLoginPublished",
                InternalEvent::SignerLoginTimedOut { .. } => "SignerLoginTimedOut",
                InternalEvent::RelayEvent(_) => "RelayEvent",
                InternalEvent::MeshEvent(_) => "MeshEvent",
                InternalEvent::FipsNearbyPacket { .. } => "FipsNearbyPacket",
                InternalEvent::FipsNearbyPeersChanged { .. } => "FipsNearbyPeersChanged",
                InternalEvent::FetchCatchUpEvents(_) => "FetchCatchUpEvents",
                InternalEvent::ProfileMetadataFetchFinished { .. } => {
                    "ProfileMetadataFetchFinished"
                }
                InternalEvent::DirectChatCapabilityFetchFinished { .. } => {
                    "DirectChatCapabilityFetchFinished"
                }
                InternalEvent::UserDiscoveryFetchFinished { .. } => "UserDiscoveryFetchFinished",
                InternalEvent::ProfileSearchRequested { .. } => "ProfileSearchRequested",
                InternalEvent::ProfileSearchDebounceElapsed { .. } => {
                    "ProfileSearchDebounceElapsed"
                }
                InternalEvent::ProfileSearchFetchFinished { .. } => "ProfileSearchFetchFinished",
                InternalEvent::ProfileSearchCapabilitiesReady { .. } => {
                    "ProfileSearchCapabilitiesReady"
                }
                InternalEvent::FetchTrackedPeerCatchUp { .. } => "FetchTrackedPeerCatchUp",
                InternalEvent::ProtocolSubscriptionLivenessCheck { .. } => {
                    "ProtocolSubscriptionLivenessCheck"
                }
                InternalEvent::PollPendingDeviceInvites { .. } => "PollPendingDeviceInvites",
                InternalEvent::PollPendingDeviceLink { .. } => "PollPendingDeviceLink",
                InternalEvent::PendingDeviceLinkRefreshFinished { .. } => {
                    "PendingDeviceLinkRefreshFinished"
                }
                InternalEvent::PruneExpiredMessages { .. } => "PruneExpiredMessages",
                InternalEvent::RelayStatusChanged { .. } => "RelayStatusChanged",
                InternalEvent::ProtocolSubscriptionReconcileCompleted { .. } => {
                    "ProtocolSubscriptionReconcileCompleted"
                }
                InternalEvent::RelayTransportConnectionFinished { .. } => {
                    "RelayTransportConnectionFinished"
                }
                #[cfg(not(target_os = "ios"))]
                InternalEvent::DebugSnapshotWriteFinished { .. } => "DebugSnapshotWriteFinished",
                InternalEvent::DebugLog { .. } => "DebugLog",
                InternalEvent::TypingIndicatorExpired { .. } => "TypingIndicatorExpired",
                InternalEvent::FlushPendingDeliveredReceipts { .. } => {
                    "FlushPendingDeliveredReceipts"
                }
                InternalEvent::RelayPublishDrainFinished { .. } => "RelayPublishDrainFinished",
                InternalEvent::RelayPublishDrainProgress { .. } => "RelayPublishDrainProgress",
                InternalEvent::SessionStartupFollowUp => "SessionStartupFollowUp",
                InternalEvent::RetryPendingRelayPublishes { .. } => "RetryPendingRelayPublishes",
                InternalEvent::AttachmentUploadFinished { .. } => "AttachmentUploadFinished",
                InternalEvent::AttachmentUploadProgress { .. } => "AttachmentUploadProgress",
                InternalEvent::ProfilePictureUploadFinished { .. } => {
                    "ProfilePictureUploadFinished"
                }
                InternalEvent::GroupPictureUploadFinished { .. } => "GroupPictureUploadFinished",
                InternalEvent::SyncComplete => "SyncComplete",
                InternalEvent::OwnerRegistrationLookupFinished { .. } => {
                    "OwnerRegistrationLookupFinished"
                }
                InternalEvent::ProtocolAuthorBackfillComplete { .. } => {
                    "ProtocolAuthorBackfillComplete"
                }
                InternalEvent::OpenChatFinalize { .. } => "OpenChatFinalize",
                InternalEvent::DeviceSyncPacket { .. } => "DeviceSyncPacket",
            },
            CoreMsg::ExportSupportBundle(_) => "ExportSupportBundle",
            CoreMsg::PeerProfileDebug { .. } => "PeerProfileDebug",
            CoreMsg::MutualGroups { .. } => "MutualGroups",
            CoreMsg::AttachHostBle { .. } => "AttachHostBle",
            CoreMsg::DetachHostBle(_) => "DetachHostBle",
            CoreMsg::CorePerfCounters(_) => "CorePerfCounters",
            CoreMsg::PrepareForSuspend(_) => "PrepareForSuspend",
            CoreMsg::Shutdown(_) => "Shutdown",
            #[cfg(test)]
            CoreMsg::PanicForTest => "PanicForTest",
        };
        match msg {
            CoreMsg::Action(action) => self.handle_action(action),
            CoreMsg::Internal(event) => self.handle_internal(*event),
            CoreMsg::ExportSupportBundle(reply_tx) => {
                let _ = reply_tx.send(self.export_support_bundle_json());
            }
            CoreMsg::PeerProfileDebug {
                owner_input,
                reply_tx,
            } => {
                self.fetch_missing_profile_metadata(&owner_input, "profile_debug");
                let _ = reply_tx.send(self.build_peer_profile_debug_snapshot(&owner_input));
            }
            CoreMsg::MutualGroups {
                owner_input,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.mutual_groups_snapshot(&owner_input));
            }
            CoreMsg::AttachHostBle {
                attachment,
                reply_tx,
            } => {
                let result = self.attach_host_ble(attachment);
                let accepted = result.is_ok();
                let _ = reply_tx.send(result);
                if accepted {
                    self.reconcile_device_sync();
                }
            }
            CoreMsg::DetachHostBle(reply_tx) => {
                self.detach_host_ble();
                let _ = reply_tx.send(());
            }
            CoreMsg::PrepareForSuspend(reply_tx) => {
                self.prepare_for_suspend();
                let _ = reply_tx.send(());
            }
            CoreMsg::CorePerfCounters(reply_tx) => {
                let _ = reply_tx.send(crate::updates::CorePerfCountersSnapshot {
                    debug_snapshot_builds: self.debug_snapshot_build_count(),
                });
            }
            CoreMsg::Shutdown(reply_tx) => {
                self.shutdown();
                if let Some(reply_tx) = reply_tx {
                    let _ = reply_tx.send(());
                }
                return false;
            }
            #[cfg(test)]
            CoreMsg::PanicForTest => {
                panic!("test core panic");
            }
        }
        crate::perflog!(
            "handle_message label={label} elapsed_ms={}",
            crate::perflog::now_ms().saturating_sub(t0)
        );
        true
    }

    /// Process a coalesced batch of messages with a single rebuild + emit at
    /// the end. Returns false if any message asked the core to shut down.
    ///
    /// The FFI message pump uses this so a burst of relay events plus user
    /// actions (e.g. tapping a chat row while events are arriving) result in
    /// one UI update instead of one per message.
    pub fn handle_messages(&mut self, messages: Vec<CoreMsg>) -> bool {
        if messages.is_empty() {
            return true;
        }
        // Always batch so a single relay event that cascades into multiple
        // engine.persist() calls (sync_group_to_local_siblings → retry_pending_group_inputs
        // → retry_pending_group_fanouts → persist) issues one engine write at exit.
        self.enter_batch();
        let mut keep_running = true;
        for msg in messages {
            if !self.handle_message(msg) {
                keep_running = false;
                break;
            }
        }
        self.exit_batch();
        keep_running
    }

    pub(super) fn shutdown(&mut self) {
        self.stop_remote_signer();
        self.finish_call("Call ended");
        self.push_debug_log("app.shutdown", "stopping core");
        self.pause_pending_linked_device();
        self.stop_device_sync();
        self.reset_pending_invite_acceptance();
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.debug_snapshot_write_generation = self.debug_snapshot_write_generation.wrapping_add(1);
        self.debug_snapshot_write_inflight = false;
        self.debug_snapshot_write_dirty = false;
        self.protocol_engine = None;
        if let Some(existing) = self.logged_in.take() {
            self.runtime.block_on(async {
                existing.client.unsubscribe_all().await;
                let _ = existing.client.shutdown().await;
            });
        }
    }

    pub(super) fn prepare_for_suspend(&mut self) {
        if self.calls.active.is_some() {
            return;
        }
        // Set the gate first so any relay/internal events that arrive in
        // the FFI queue after this point (whether already-queued or sent
        // by an in-flight tokio task before disconnect lands) are dropped
        // without touching SQLite. The persist below is the only write we
        // want before iOS suspends us.
        self.suspended = true;
        self.push_debug_log("app.suspend", "pausing network and flushing storage");
        self.pause_pending_linked_device();
        self.stop_device_sync();
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.saturating_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_transport_runtime = RelayTransportRuntime::default();
        self.cancel_people_fetches_for_suspend();
        self.reset_direct_chat_capability_runtime();
        self.profile_metadata_fetch_inflight.clear();
        self.pending_relay_publish_inflight.clear();
        self.relay_connected_count = 0;
        self.all_relays_offline_since_secs = None;
        self.debug_snapshot_write_generation = self.debug_snapshot_write_generation.wrapping_add(1);
        self.debug_snapshot_write_inflight = false;
        self.debug_snapshot_write_dirty = false;
        self.state.busy.syncing_network = false;
        self.persist_best_effort();

        if let Some(logged_in) = self.logged_in.as_ref() {
            let client = logged_in.client.clone();
            self.runtime.block_on(async move {
                let _ = tokio::time::timeout(Duration::from_millis(750), async move {
                    client.unsubscribe_all().await;
                    client.disconnect().await;
                })
                .await;
            });
        }

        if let Err(error) = self.app_store.prepare_for_suspend() {
            self.push_debug_log("storage.suspend.error", error.to_string());
        }
        self.rebuild_state();
        self.emit_state();
    }
    pub(super) fn handle_internal(&mut self, event: InternalEvent) {
        if self.suspended {
            self.defer_remote_signer_event(event);
            // Drop queued background work while iOS is taking us down. We
            // don't want any further SQLite writes once the suspend
            // checkpoint has run. Foregrounding clears the gate and
            // re-establishes subscriptions so dropped events are re-fetched.
            return;
        }
        match event {
            event @ (InternalEvent::RemoteSignerProgress { .. }
            | InternalEvent::RemoteSignerConnected { .. }
            | InternalEvent::RemoteSignerSigned { .. }
            | InternalEvent::RemoteSignerFailed { .. }) => self.handle_remote_signer_event(event),
            InternalEvent::CallPacket {
                source_pubkey_hex,
                source_port,
                data,
            } => self.handle_call_packet(&source_pubkey_hex, source_port, &data),
            InternalEvent::CallTick { call_id } => self.call_tick(&call_id),
            InternalEvent::CallRecoveryTick { call_id } => self.call_recovery_tick(&call_id),
            InternalEvent::SignerLoginFetched { request_id, result } => {
                self.handle_signer_login_fetched(&request_id, result)
            }
            InternalEvent::SignerLoginPublished { request_id, result } => {
                self.handle_signer_login_published(&request_id, result)
            }
            InternalEvent::SignerLoginTimedOut { request_id } => {
                self.handle_signer_login_timeout(&request_id)
            }
            InternalEvent::RelayEvent(event) => {
                self.handle_relay_event_with_channel(event, "message servers");
            }
            InternalEvent::MeshEvent(event) => {
                self.handle_relay_event_with_channel(event, "FIPS mesh");
            }
            InternalEvent::FipsNearbyPacket {
                source_pubkey_hex,
                source_port,
                data,
            } => {
                self.handle_fips_nearby_packet(&source_pubkey_hex, source_port, &data);
            }
            InternalEvent::FipsNearbyPeersChanged { generation, peers } => {
                if generation == self.fips_connection_generation {
                    self.update_fips_connection_links(peers);
                }
            }
            InternalEvent::FetchTrackedPeerCatchUp { token } => {
                if token
                    != self
                        .protocol_subscription_runtime
                        .tracked_peer_catch_up_token
                {
                    return;
                }
                self.protocol_subscription_runtime
                    .tracked_peer_catch_up_due_at = None;
                let should_fetch_tracked_peer_messages =
                    self.protocol_subscription_runtime.desired_plan
                        != self.protocol_subscription_runtime.applied_plan
                        || self.protocol_subscription_runtime.refresh_dirty
                        || self.message_recipient_bootstrap_needed();
                self.push_debug_log(
                    "protocol.catch_up.schedule",
                    format!("fetch tracked peers messages={should_fetch_tracked_peer_messages}"),
                );
                self.fetch_recent_protocol_metadata_state();
                if should_fetch_tracked_peer_messages {
                    self.fetch_recent_messages_for_tracked_peers();
                }
                self.retry_protocol_engine_pending_work("tracked_peer_catch_up");
                if self.is_device_roster_open() {
                    self.fetch_pending_device_invites_for_local_owner();
                }
            }
            InternalEvent::ProtocolSubscriptionLivenessCheck { token } => {
                self.handle_protocol_subscription_liveness_check(token);
            }
            InternalEvent::PollPendingDeviceInvites { token } => {
                if token != self.device_invite_poll_token || !self.can_poll_pending_device_invites()
                {
                    return;
                }
                self.fetch_pending_device_invites_for_local_owner();
                self.schedule_pending_device_invite_poll(Duration::from_secs(
                    DEVICE_INVITE_DISCOVERY_POLL_SECS,
                ));
            }
            InternalEvent::PollPendingDeviceLink { token } => {
                if token != self.pending_device_link_poll_token {
                    return;
                }
                self.refresh_pending_linked_device();
            }
            InternalEvent::PendingDeviceLinkRefreshFinished { token, events } => {
                if token != self.pending_device_link_poll_token {
                    return;
                }
                if let Some(pending) = self.pending_linked_device.as_mut() {
                    pending.refresh_in_flight = false;
                }
                for event in events {
                    self.handle_relay_event_with_channel(event, "device approval server");
                    if self.pending_linked_device.is_none() {
                        return;
                    }
                }
                self.schedule_pending_linked_device_refresh(Duration::from_secs(
                    PENDING_DEVICE_LINK_RETRY_SECS,
                ));
            }
            InternalEvent::PruneExpiredMessages { token } => {
                self.handle_prune_expired_messages(token);
            }
            InternalEvent::FetchCatchUpEvents(mut events) => {
                // Coalesce: a catch-up burst of N events used to cause N
                // rebuild_state + emit_state cycles, each pushing a fresh
                // FullState to the UI. On Android debug builds that meant
                // 16-19 recompositions in a row whenever the relay flushed
                // a backlog and the screen could be unresponsive for
                // seconds. Process bounded chunks inside batches so the UI
                // still gets coalesced updates without starving user actions.
                let remainder = if events.len() > CATCH_UP_EVENT_PROCESS_CHUNK_SIZE {
                    Some(events.split_off(CATCH_UP_EVENT_PROCESS_CHUNK_SIZE))
                } else {
                    None
                };
                self.enter_batch();
                for event in events {
                    self.handle_relay_event(event);
                }
                self.exit_batch();
                if let Some(remainder) = remainder {
                    let _ = self.core_sender.send(CoreMsg::Internal(Box::new(
                        InternalEvent::FetchCatchUpEvents(remainder),
                    )));
                }
            }
            InternalEvent::ProfileMetadataFetchFinished {
                owner_pubkey_hex,
                events,
                error,
            } => {
                self.profile_metadata_fetch_inflight
                    .remove(&owner_pubkey_hex);
                if let Some(error) = error {
                    self.push_debug_log(
                        "profile.metadata.fetch.error",
                        format!("owner={owner_pubkey_hex} error={error}"),
                    );
                    return;
                }
                self.push_debug_log(
                    "profile.metadata.fetch.result",
                    format!("owner={owner_pubkey_hex} events={}", events.len()),
                );
                if events.is_empty() {
                    return;
                }
                self.enter_batch();
                for event in events {
                    self.handle_relay_event(event);
                }
                self.exit_batch();
            }
            InternalEvent::DirectChatCapabilityFetchFinished {
                generation,
                token,
                owner_pubkey_hex,
                result,
            } => self.handle_direct_chat_capability_fetch_finished(
                generation,
                token,
                &owner_pubkey_hex,
                result,
            ),
            InternalEvent::UserDiscoveryFetchFinished { token, result } => {
                self.handle_user_discovery_fetch_finished(token, result);
            }
            InternalEvent::ProfileSearchRequested { query } => {
                self.request_profile_search(&query);
            }
            InternalEvent::ProfileSearchDebounceElapsed { token, query } => {
                self.handle_profile_search_debounce_elapsed(token, &query);
            }
            InternalEvent::ProfileSearchFetchFinished {
                token,
                query,
                result,
            } => {
                self.handle_profile_search_fetch_finished(token, &query, result);
            }
            InternalEvent::ProfileSearchCapabilitiesReady {
                token,
                query,
                events,
            } => {
                self.handle_profile_search_capabilities_ready(token, &query, events);
            }
            InternalEvent::RelayStatusChanged {
                relay_url,
                status,
                generation,
            } => {
                self.handle_relay_status_changed_for_generation(relay_url, status, generation);
            }
            InternalEvent::ProtocolSubscriptionReconcileCompleted {
                generation,
                token,
                reason,
                plan,
                success,
                error,
                relay_statuses,
                connected_before,
                connected_after,
                filter_count,
            } => {
                self.handle_protocol_subscription_reconcile_completed(
                    generation,
                    token,
                    reason,
                    plan,
                    success,
                    error,
                    relay_statuses,
                    connected_before,
                    connected_after,
                    filter_count,
                );
            }
            InternalEvent::RelayTransportConnectionFinished {
                token,
                reason,
                relay_statuses,
                connected_count,
            } => {
                self.handle_relay_transport_connection_finished(
                    token,
                    reason,
                    relay_statuses,
                    connected_count,
                );
            }
            #[cfg(not(target_os = "ios"))]
            InternalEvent::DebugSnapshotWriteFinished { generation } => {
                self.handle_debug_snapshot_write_finished(generation);
            }
            InternalEvent::DebugLog { category, detail } => {
                self.push_debug_log(&category, detail);
                self.persist_debug_snapshot_best_effort();
            }
            InternalEvent::TypingIndicatorExpired { chat_id, author } => {
                let key = format!("{chat_id}\n{author}");
                let should_remove = self
                    .typing_indicators
                    .get(&key)
                    .map(|indicator| indicator.expires_at_secs <= unix_now().get())
                    .unwrap_or(false);
                if should_remove {
                    self.typing_indicators.remove(&key);
                    self.rebuild_state();
                    self.emit_state();
                }
            }
            InternalEvent::FlushPendingDeliveredReceipts { token } => {
                self.handle_pending_delivered_receipt_flush(token);
            }
            InternalEvent::RelayPublishDrainFinished { token, results } => {
                self.handle_relay_publish_drain_finished(token, results);
            }
            InternalEvent::RelayPublishDrainProgress { token, result } => {
                self.handle_relay_publish_drain_progress(token, result);
            }
            InternalEvent::SessionStartupFollowUp => {
                self.run_session_startup_follow_up();
            }
            InternalEvent::RetryPendingRelayPublishes { reason } => {
                self.retry_pending_relay_publishes(&reason);
            }
            InternalEvent::AttachmentUploadFinished { chat_id, result } => {
                self.handle_attachment_upload_finished(chat_id, result);
            }
            InternalEvent::AttachmentUploadProgress {
                bytes_uploaded,
                total_bytes,
            } => {
                self.handle_attachment_upload_progress(bytes_uploaded, total_bytes);
            }
            InternalEvent::ProfilePictureUploadFinished { result } => {
                self.handle_profile_picture_upload_finished(result);
            }
            InternalEvent::GroupPictureUploadFinished { group_id, result } => {
                self.handle_group_picture_upload_finished(group_id, result);
            }
            InternalEvent::OwnerRegistrationLookupFinished {
                generation,
                owner,
                device,
                completed,
                queried,
                events,
            } => {
                self.complete_owner_registration_lookup(
                    generation, owner, device, completed, queried, events,
                );
            }
            InternalEvent::SyncComplete => {
                self.recover_deferred_owner_registration();
                self.protocol_subscription_runtime.protocol_fetch_in_flight = false;
                self.refresh_protocol_sync_busy();
                self.rebuild_state();
                self.emit_state();
            }
            InternalEvent::ProtocolAuthorBackfillComplete { reason } => {
                self.protocol_subscription_runtime
                    .protocol_author_backfill_in_flight = self
                    .protocol_subscription_runtime
                    .protocol_author_backfill_in_flight
                    .saturating_sub(1);
                self.push_debug_log(
                    "protocol.author_backfill.complete",
                    format!(
                        "reason={reason} remaining={}",
                        self.protocol_subscription_runtime
                            .protocol_author_backfill_in_flight
                    ),
                );
                self.refresh_protocol_sync_busy();
                self.rebuild_state();
                self.emit_state();
            }
            InternalEvent::OpenChatFinalize { chat_id } => {
                self.open_chat_finalize(&chat_id);
            }
            InternalEvent::DeviceSyncPacket {
                source_pubkey_hex,
                source_port,
                data,
            } => self.handle_device_sync_packet(&source_pubkey_hex, source_port, &data),
        }
    }
}
