use super::*;

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
            protocol_subscription_runtime: ProtocolSubscriptionRuntime::default(),
            direct_message_subscriptions: DirectMessageSubscriptionTracker::new(),
            relay_status_watch_urls: HashSet::new(),
            relay_connected_count: 0,
            all_relays_offline_since_secs: None,
            pending_relay_publishes: BTreeMap::new(),
            pending_relay_publish_inflight: HashSet::new(),
            pending_decrypted_delivery_acks: HashSet::new(),
            event_transport_channels: BTreeMap::new(),
            pending_mobile_push_events: VecDeque::new(),
            debug_log: VecDeque::new(),
            debug_event_counters: DebugEventCounters::default(),
            batch_depth: 0,
            batch_dirty_state: false,
            batch_dirty_persist: false,
            last_emitted_state: None,
            app_store,
            _data_dir_lock: data_dir_lock,
            cached_mobile_push: MobilePushSyncSnapshot::default(),
            // First rebuild populates the cache.
            mobile_push_dirty: true,
        })
    }

    pub fn handle_message(&mut self, msg: CoreMsg) -> bool {
        let t0 = crate::perflog::now_ms();
        let label: &'static str = match &msg {
            CoreMsg::Action(action) => match action {
                AppAction::OpenChat { .. } => "OpenChat",
                AppAction::SendMessage { .. } => "SendMessage",
                AppAction::PushScreen { .. } => "PushScreen",
                AppAction::UpdateScreenStack { .. } => "UpdateScreenStack",
                AppAction::AppForegrounded => "AppForegrounded",
                AppAction::MarkMessagesSeen { .. } => "MarkMessagesSeen",
                _ => "Action.other",
            },
            CoreMsg::Internal(event) => match event.as_ref() {
                InternalEvent::RelayEvent(_) => "RelayEvent",
                InternalEvent::NearbyEvent { .. } => "NearbyEvent",
                InternalEvent::FetchCatchUpEvents(_) => "FetchCatchUpEvents",
                InternalEvent::FetchTrackedPeerCatchUp => "FetchTrackedPeerCatchUp",
                InternalEvent::ProtocolSubscriptionLivenessCheck { .. } => {
                    "ProtocolSubscriptionLivenessCheck"
                }
                InternalEvent::PollPendingDeviceInvites { .. } => "PollPendingDeviceInvites",
                InternalEvent::PruneExpiredMessages { .. } => "PruneExpiredMessages",
                InternalEvent::RelayStatusChanged { .. } => "RelayStatusChanged",
                InternalEvent::RelayConnectionChecked { .. } => "RelayConnectionChecked",
                InternalEvent::DebugLog { .. } => "DebugLog",
                InternalEvent::TypingIndicatorExpired { .. } => "TypingIndicatorExpired",
                InternalEvent::RelayPublishFinished { .. } => "RelayPublishFinished",
                InternalEvent::RetryPendingRelayPublishes { .. } => "RetryPendingRelayPublishes",
                InternalEvent::AttachmentUploadFinished { .. } => "AttachmentUploadFinished",
                InternalEvent::ProfilePictureUploadFinished { .. } => {
                    "ProfilePictureUploadFinished"
                }
                InternalEvent::SyncComplete => "SyncComplete",
            },
            CoreMsg::BuildNearbyPresenceEvent { .. } => "BuildNearbyPresenceEvent",
            CoreMsg::ExportSupportBundle(_) => "ExportSupportBundle",
            CoreMsg::PeerProfileDebug { .. } => "PeerProfileDebug",
            CoreMsg::PrepareForSuspend(_) => "PrepareForSuspend",
            CoreMsg::Shutdown(_) => "Shutdown",
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
            CoreMsg::PrepareForSuspend(reply_tx) => {
                self.prepare_for_suspend();
                let _ = reply_tx.send(());
            }
            CoreMsg::Shutdown(reply_tx) => {
                self.shutdown();
                if let Some(reply_tx) = reply_tx {
                    let _ = reply_tx.send(());
                }
                return false;
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
            return self.handle_message(messages.into_iter().next().unwrap());
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
        self.relay_status_watch_urls.clear();
        self.protocol_engine = None;
        if let Some(existing) = self.logged_in.take() {
            self.runtime.block_on(async {
                existing.client.unsubscribe_all().await;
                let _ = existing.client.shutdown().await;
            });
        }
    }

    pub(super) fn prepare_for_suspend(&mut self) {
        self.push_debug_log("app.suspend", "pausing network and flushing storage");
        self.stop_pending_linked_device();
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.saturating_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.relay_status_watch_urls.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_connected_count = 0;
        self.all_relays_offline_since_secs = None;
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
            AppAction::RestoreSession { owner_nsec } => self.restore_primary_session(&owner_nsec),
            AppAction::RestoreAccountBundle {
                owner_nsec,
                owner_pubkey_hex,
                device_nsec,
            } => self.restore_account_bundle(owner_nsec, &owner_pubkey_hex, &device_nsec),
            AppAction::StartLinkedDevice { owner_input } => self.start_linked_device(&owner_input),
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
            AppAction::SetNearbyBluetoothEnabled { enabled } => {
                self.set_nearby_bluetooth_enabled(enabled)
            }
            AppAction::SetNearbyLanEnabled { enabled } => self.set_nearby_lan_enabled(enabled),
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
            AppAction::UpdateScreenStack { stack } => self.update_screen_stack(stack),
        }
    }

    pub(super) fn handle_internal(&mut self, event: InternalEvent) {
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
            InternalEvent::FetchTrackedPeerCatchUp => {
                let now = unix_now();
                self.push_debug_log("protocol.catch_up.schedule", "fetch tracked peers");
                self.fetch_recent_protocol_state();
                self.fetch_recent_messages_for_tracked_peers(now);
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
            InternalEvent::FetchCatchUpEvents(events) => {
                // Coalesce: a catch-up burst of N events used to cause N
                // rebuild_state + emit_state cycles, each pushing a fresh
                // FullState to the UI. On Android debug builds that meant
                // 16-19 recompositions in a row whenever the relay flushed
                // a backlog and the screen could be unresponsive for
                // seconds. Process all events inside one batch so the UI
                // sees a single update at the end.
                self.enter_batch();
                for event in events {
                    self.handle_relay_event(event);
                }
                self.exit_batch();
            }
            InternalEvent::RelayStatusChanged { relay_url, status } => {
                self.handle_relay_status_changed(relay_url, status);
            }
            InternalEvent::RelayConnectionChecked { reason } => {
                self.handle_relay_connection_checked(reason);
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
            InternalEvent::RelayPublishFinished {
                event_id,
                message_id,
                chat_id,
                success,
                relay_urls,
                detail,
            } => {
                self.handle_relay_publish_finished(
                    event_id, message_id, chat_id, success, relay_urls, detail,
                );
            }
            InternalEvent::RetryPendingRelayPublishes { reason } => {
                self.retry_pending_relay_publishes(&reason);
            }
            InternalEvent::AttachmentUploadFinished { chat_id, result } => {
                self.handle_attachment_upload_finished(chat_id, result);
            }
            InternalEvent::ProfilePictureUploadFinished { result } => {
                self.handle_profile_picture_upload_finished(result);
            }
            InternalEvent::SyncComplete => {
                self.state.busy.syncing_network = false;
                self.rebuild_state();
                self.emit_state();
            }
        }
    }
}
