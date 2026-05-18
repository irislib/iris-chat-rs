use super::*;

pub(super) const CATCH_UP_EVENT_PROCESS_CHUNK_SIZE: usize = 64;

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

    pub fn try_new(
        update_tx: Sender<AppUpdate>,
        core_sender: Sender<CoreMsg>,
        data_dir: String,
        shared_state: Arc<RwLock<AppState>>,
    ) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .max_blocking_threads(8)
            .build()?;

        let state = AppState::empty();
        match shared_state.write() {
            Ok(mut slot) => *slot = state.clone(),
            Err(poison) => *poison.into_inner() = state.clone(),
        }

        let data_dir = PathBuf::from(data_dir);
        let data_dir_lock = DataDirLock::acquire(&data_dir)?;
        let app_store = AppStore::new(open_database(&data_dir)?);

        Ok(Self {
            update_tx,
            core_sender,
            shared_state,
            runtime,
            data_dir,
            state: state.clone(),
            logged_in: None,
            protocol_engine: None,
            pending_linked_device: None,
            private_chat_invites: BTreeMap::new(),
            threads: BTreeMap::new(),
            active_chat_id: None,
            screen_stack: Vec::new(),
            next_message_id: 1,
            owner_profiles: BTreeMap::new(),
            app_keys: BTreeMap::new(),
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
            message_expiry_token: 0,
            protocol_reconnect_token: 0,
            defer_owner_app_keys_publish: false,
            current_device_labels: None,
            protocol_subscription_runtime: ProtocolSubscriptionRuntime::default(),
            relay_transport_runtime: RelayTransportRuntime::default(),
            relay_status_watch_urls: HashSet::new(),
            relay_status_watch_generation: 0,
            relay_status_by_url: BTreeMap::new(),
            relay_connected_count: 0,
            all_relays_offline_since_secs: None,
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
                InternalEvent::RelayEvent(_) => "RelayEvent",
                InternalEvent::NearbyEvent { .. } => "NearbyEvent",
                InternalEvent::FetchCatchUpEvents(_) => "FetchCatchUpEvents",
                InternalEvent::FetchTrackedPeerCatchUp { .. } => "FetchTrackedPeerCatchUp",
                InternalEvent::ProtocolSubscriptionLivenessCheck { .. } => {
                    "ProtocolSubscriptionLivenessCheck"
                }
                InternalEvent::PollPendingDeviceInvites { .. } => "PollPendingDeviceInvites",
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
                InternalEvent::RelayPublishDrainFinished { .. } => "RelayPublishDrainFinished",
                InternalEvent::RetryPendingRelayPublishes { .. } => "RetryPendingRelayPublishes",
                InternalEvent::AttachmentUploadFinished { .. } => "AttachmentUploadFinished",
                InternalEvent::AttachmentUploadProgress { .. } => "AttachmentUploadProgress",
                InternalEvent::ProfilePictureUploadFinished { .. } => {
                    "ProfilePictureUploadFinished"
                }
                InternalEvent::GroupPictureUploadFinished { .. } => "GroupPictureUploadFinished",
                InternalEvent::SyncComplete => "SyncComplete",
                InternalEvent::OpenChatFinalize { .. } => "OpenChatFinalize",
            },
            CoreMsg::BuildNearbyPresenceEvent { .. } => "BuildNearbyPresenceEvent",
            CoreMsg::ExportSupportBundle(_) => "ExportSupportBundle",
            CoreMsg::PeerProfileDebug { .. } => "PeerProfileDebug",
            CoreMsg::MutualGroups { .. } => "MutualGroups",
            CoreMsg::CorePerfCounters(_) => "CorePerfCounters",
            CoreMsg::PrepareForSuspend(_) => "PrepareForSuspend",
            CoreMsg::Shutdown(_) => "Shutdown",
            #[cfg(test)]
            CoreMsg::PanicForTest => "PanicForTest",
        };
        match msg {
            CoreMsg::Action(action) => self.handle_action(action),
            CoreMsg::Internal(event) => self.handle_internal(*event),
            CoreMsg::BuildNearbyPresenceEvent {
                peer_id,
                my_nonce,
                their_nonce,
                profile_event_id,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.build_nearby_presence_event_json(
                    &peer_id,
                    &my_nonce,
                    &their_nonce,
                    &profile_event_id,
                ));
            }
            CoreMsg::ExportSupportBundle(reply_tx) => {
                let _ = reply_tx.send(self.export_support_bundle_json());
            }
            CoreMsg::PeerProfileDebug {
                owner_input,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.build_peer_profile_debug_snapshot(&owner_input));
            }
            CoreMsg::MutualGroups {
                owner_input,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.mutual_groups_snapshot(&owner_input));
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
        if messages.len() == 1 {
            let Some(message) = messages.into_iter().next() else {
                return true;
            };
            return self.handle_message(message);
        }
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
        self.push_debug_log("app.shutdown", "stopping core");
        self.stop_pending_linked_device();
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
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
        // Set the gate first so any relay/internal events that arrive in
        // the FFI queue after this point (whether already-queued or sent
        // by an in-flight tokio task before disconnect lands) are dropped
        // without touching SQLite. The persist below is the only write we
        // want before iOS suspends us.
        self.suspended = true;
        self.push_debug_log("app.suspend", "pausing network and flushing storage");
        self.stop_pending_linked_device();
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.saturating_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_transport_runtime = RelayTransportRuntime::default();
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

    pub(super) fn handle_action(&mut self, action: AppAction) {
        self.state.toast = None;
        match action {
            AppAction::CreateAccount { name } => self.create_account(&name),
            AppAction::UpdateProfileMetadata { name, picture_url } => {
                self.update_profile_metadata(&name, picture_url.as_deref())
            }
            AppAction::SetContactNickname {
                owner_pubkey_hex,
                nickname,
            } => self.set_contact_nickname(&owner_pubkey_hex, &nickname),
            AppAction::DeleteProfileMetadata => self.delete_profile_metadata(),
            AppAction::RestoreSession { owner_nsec } => self.restore_primary_session(&owner_nsec),
            AppAction::RestoreAccountBundle {
                owner_nsec,
                owner_pubkey_hex,
                device_nsec,
            } => self.restore_account_bundle(owner_nsec, &owner_pubkey_hex, &device_nsec),
            AppAction::StartLinkedDevice { owner_input } => self.start_linked_device(&owner_input),
            AppAction::SetCurrentDeviceLabels {
                device_label,
                client_label,
            } => self.set_current_device_labels(&device_label, &client_label),
            AppAction::AppForegrounded => self.handle_app_foregrounded(),
            AppAction::Logout => self.logout(),
            AppAction::CreateChat { peer_input } => self.create_chat(&peer_input),
            AppAction::CreateGroup {
                name,
                member_inputs,
            } => self.create_group(&name, &member_inputs),
            AppAction::CreateGroupWithPicture {
                name,
                member_inputs,
                picture_file_path,
                picture_filename,
            } => self.create_group_with_picture(
                &name,
                &member_inputs,
                &picture_file_path,
                &picture_filename,
            ),
            AppAction::CreatePublicInvite => self.create_public_invite(),
            AppAction::AcceptInvite { invite_input } => self.accept_invite(&invite_input),
            AppAction::OpenChat { chat_id } => self.open_chat(&chat_id),
            AppAction::SendMessage { chat_id, text } => self.send_message(&chat_id, &text, None),
            AppAction::SendDisappearingMessage {
                chat_id,
                text,
                expires_at_secs,
            } => self.send_message(&chat_id, &text, Some(expires_at_secs)),
            AppAction::SetChatMessageTtl {
                chat_id,
                ttl_seconds,
            } => self.set_chat_message_ttl(&chat_id, ttl_seconds),
            AppAction::SetChatMuted { chat_id, muted } => self.set_chat_muted(&chat_id, muted),
            AppAction::SetChatPinned { chat_id, pinned } => self.set_chat_pinned(&chat_id, pinned),
            AppAction::SetChatUnread { chat_id, unread } => self.set_chat_unread(&chat_id, unread),
            AppAction::SendAttachment {
                chat_id,
                file_path,
                filename,
                caption,
            } => self.send_attachment(&chat_id, &file_path, &filename, &caption),
            AppAction::SendAttachments {
                chat_id,
                attachments,
                caption,
            } => self.send_attachments(&chat_id, &attachments, &caption),
            AppAction::ToggleReaction {
                chat_id,
                message_id,
                emoji,
            } => self.toggle_reaction(&chat_id, &message_id, &emoji),
            AppAction::SendTyping { chat_id } => self.send_typing(&chat_id),
            AppAction::StopTyping { chat_id } => self.stop_typing(&chat_id),
            AppAction::SetTypingIndicatorsEnabled { enabled } => {
                self.set_typing_indicators_enabled(enabled)
            }
            AppAction::SetReadReceiptsEnabled { enabled } => {
                self.set_read_receipts_enabled(enabled)
            }
            AppAction::SetDesktopNotificationsEnabled { enabled } => {
                self.set_desktop_notifications_enabled(enabled)
            }
            AppAction::SetInviteAcceptanceNotificationsEnabled { enabled } => {
                self.set_invite_acceptance_notifications_enabled(enabled)
            }
            AppAction::SetStartupAtLoginEnabled { enabled } => {
                self.set_startup_at_login_enabled(enabled)
            }
            AppAction::SetNearbyEnabled { enabled } => self.set_nearby_enabled(enabled),
            AppAction::SetNearbyBluetoothEnabled { enabled } => {
                self.set_nearby_bluetooth_enabled(enabled)
            }
            AppAction::SetNearbyLanEnabled { enabled } => self.set_nearby_lan_enabled(enabled),
            AppAction::SetDebugLoggingEnabled { enabled } => {
                self.set_debug_logging_enabled(enabled)
            }
            AppAction::SetAcceptUnknownDirectMessages { enabled } => {
                self.set_accept_unknown_direct_messages(enabled)
            }
            AppAction::SetUserBlocked {
                owner_pubkey_hex,
                blocked,
            } => self.set_user_blocked(&owner_pubkey_hex, blocked),
            AppAction::SetMessageRequestAccepted { chat_id } => {
                self.accept_message_request(&chat_id)
            }
            AppAction::SetNearbyMailbagEnabled { enabled } => {
                self.set_nearby_mailbag_enabled(enabled)
            }
            AppAction::AddNostrRelay { relay_url } => self.add_nostr_relay(&relay_url),
            AppAction::UpdateNostrRelay {
                old_relay_url,
                new_relay_url,
            } => self.update_nostr_relay(&old_relay_url, &new_relay_url),
            AppAction::RemoveNostrRelay { relay_url } => self.remove_nostr_relay(&relay_url),
            AppAction::SetNostrRelays { relay_urls } => self.set_nostr_relays(&relay_urls),
            AppAction::ResetNostrRelays => self.reset_nostr_relays(),
            AppAction::SetImageProxyEnabled { enabled } => self.set_image_proxy_enabled(enabled),
            AppAction::SetImageProxyUrl { url } => self.set_image_proxy_url(&url),
            AppAction::SetImageProxyKeyHex { key_hex } => self.set_image_proxy_key_hex(&key_hex),
            AppAction::SetImageProxySaltHex { salt_hex } => {
                self.set_image_proxy_salt_hex(&salt_hex)
            }
            AppAction::ResetImageProxySettings => self.reset_image_proxy_settings(),
            AppAction::SetMobilePushServerUrl { url } => self.set_mobile_push_server_url(&url),
            AppAction::ResetMobilePushServerUrl => self.reset_mobile_push_server_url(),
            AppAction::IngestMobilePushPayload { payload_json } => {
                self.ingest_mobile_push_payload(&payload_json)
            }
            AppAction::MarkMessagesSeen {
                chat_id,
                message_ids,
            } => self.mark_messages_seen(&chat_id, &message_ids),
            AppAction::SendReceipt {
                chat_id,
                receipt_type,
                message_ids,
            } => self.send_receipt(&chat_id, &receipt_type, message_ids),
            AppAction::DeleteLocalMessage {
                chat_id,
                message_id,
            } => self.delete_local_message(&chat_id, &message_id),
            AppAction::DeleteChat { chat_id } => self.delete_chat(&chat_id),
            AppAction::UpdateGroupName { group_id, name } => {
                self.update_group_name(&group_id, &name)
            }
            AppAction::UpdateGroupPicture {
                group_id,
                file_path,
                filename,
            } => self.update_group_picture(&group_id, &file_path, &filename),
            AppAction::AddGroupMembers {
                group_id,
                member_inputs,
            } => self.add_group_members(&group_id, &member_inputs),
            AppAction::SetGroupAdmin {
                group_id,
                owner_pubkey_hex,
                is_admin,
            } => self.set_group_admin(&group_id, &owner_pubkey_hex, is_admin),
            AppAction::RemoveGroupMember {
                group_id,
                owner_pubkey_hex,
            } => self.remove_group_member(&group_id, &owner_pubkey_hex),
            AppAction::UploadProfilePicture { file_path } => {
                self.upload_profile_picture(&file_path)
            }
            AppAction::AddAuthorizedDevice { device_input } => {
                self.add_authorized_device(&device_input)
            }
            AppAction::RemoveAuthorizedDevice { device_pubkey_hex } => {
                self.remove_authorized_device(&device_pubkey_hex)
            }
            AppAction::AcknowledgeRevokedDevice => self.acknowledge_revoked_device(),
            AppAction::PushScreen { screen } => self.push_screen(screen),
            AppAction::NavigateBack => self.navigate_back(),
            AppAction::UpdateScreenStack { stack } => self.update_screen_stack(stack),
            AppAction::SetChatDraft { chat_id, text } => self.set_chat_draft(&chat_id, &text),
        }
    }

    pub(super) fn handle_internal(&mut self, event: InternalEvent) {
        if self.suspended {
            // Drop queued background work while iOS is taking us down. We
            // don't want any further SQLite writes once the suspend
            // checkpoint has run. Foregrounding clears the gate and
            // re-establishes subscriptions so dropped events are re-fetched.
            return;
        }
        match event {
            InternalEvent::RelayEvent(event) => {
                self.handle_relay_event_with_channel(event, "message servers");
            }
            InternalEvent::NearbyEvent { event, transport } => {
                let event_id = event.id.to_string();
                let kind = event.kind.as_u16() as u32;
                self.push_debug_log(
                    "nearby.event",
                    format!("kind_raw={kind} id={event_id} transport={transport}"),
                );
                let channel: &str = if transport.is_empty() {
                    "nearby"
                } else {
                    &transport
                };
                self.handle_relay_event_with_channel(event, channel);
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
                self.push_debug_log("protocol.catch_up.schedule", "fetch tracked peers");
                self.fetch_recent_protocol_state();
                self.fetch_recent_messages_for_tracked_peers();
                self.retry_protocol_engine_pending_outbound("tracked_peer_catch_up");
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
            InternalEvent::RelayPublishDrainFinished { token, results } => {
                self.handle_relay_publish_drain_finished(token, results);
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
            InternalEvent::SyncComplete => {
                self.protocol_subscription_runtime.protocol_fetch_in_flight = false;
                self.state.busy.syncing_network = false;
                self.rebuild_state();
                self.emit_state();
            }
            InternalEvent::OpenChatFinalize { chat_id } => {
                self.open_chat_finalize(&chat_id);
            }
        }
    }
}
