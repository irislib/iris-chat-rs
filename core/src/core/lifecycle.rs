use super::*;

impl AppCore {
    pub fn new(
        update_tx: Sender<AppUpdate>,
        core_sender: Sender<CoreMsg>,
        data_dir: String,
        shared_state: Arc<RwLock<AppState>>,
    ) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let state = AppState::empty();
        match shared_state.write() {
            Ok(mut slot) => *slot = state.clone(),
            Err(poison) => *poison.into_inner() = state.clone(),
        }

        Self {
            update_tx,
            core_sender,
            shared_state,
            runtime,
            data_dir: PathBuf::from(data_dir),
            state: state.clone(),
            logged_in: None,
            threads: BTreeMap::new(),
            active_chat_id: None,
            screen_stack: Vec::new(),
            next_message_id: 1,
            pending_inbound: Vec::new(),
            pending_outbound: Vec::new(),
            pending_group_controls: Vec::new(),
            owner_profiles: BTreeMap::new(),
            typing_indicators: BTreeMap::new(),
            preferences: state.preferences.clone(),
            recent_handshake_peers: BTreeMap::new(),
            seen_event_ids: HashSet::new(),
            seen_event_order: VecDeque::new(),
            device_invite_poll_token: 0,
            protocol_subscription_runtime: ProtocolSubscriptionRuntime::default(),
            debug_log: VecDeque::new(),
            debug_event_counters: DebugEventCounters::default(),
        }
    }

    pub fn handle_message(&mut self, msg: CoreMsg) -> bool {
        match msg {
            CoreMsg::Action(action) => self.handle_action(action),
            CoreMsg::Internal(event) => self.handle_internal(*event),
            CoreMsg::ExportSupportBundle(reply_tx) => {
                let _ = reply_tx.send(self.export_support_bundle_json());
            }
            CoreMsg::Shutdown(reply_tx) => {
                self.shutdown();
                if let Some(reply_tx) = reply_tx {
                    let _ = reply_tx.send(());
                }
                return false;
            }
        }
        true
    }

    pub(super) fn shutdown(&mut self) {
        self.push_debug_log("app.shutdown", "stopping core");
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        if let Some(existing) = self.logged_in.take() {
            self.runtime.block_on(async {
                existing.client.unsubscribe_all().await;
                let _ = existing.client.shutdown().await;
            });
        }
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
            AppAction::CreatePublicInvite => self.create_public_invite(),
            AppAction::AcceptInvite { invite_input } => self.accept_invite(&invite_input),
            AppAction::OpenChat { chat_id } => self.open_chat(&chat_id),
            AppAction::SendMessage { chat_id, text } => self.send_message(&chat_id, &text),
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
            AppAction::SetTypingIndicatorsEnabled { enabled } => {
                self.set_typing_indicators_enabled(enabled)
            }
            AppAction::SetReadReceiptsEnabled { enabled } => {
                self.set_read_receipts_enabled(enabled)
            }
            AppAction::SetDesktopNotificationsEnabled { enabled } => {
                self.set_desktop_notifications_enabled(enabled)
            }
            AppAction::SetStartupAtLoginEnabled { enabled } => {
                self.set_startup_at_login_enabled(enabled)
            }
            AppAction::AddNostrRelay { relay_url } => self.add_nostr_relay(&relay_url),
            AppAction::UpdateNostrRelay {
                old_relay_url,
                new_relay_url,
            } => self.update_nostr_relay(&old_relay_url, &new_relay_url),
            AppAction::RemoveNostrRelay { relay_url } => self.remove_nostr_relay(&relay_url),
            AppAction::ResetNostrRelays => self.reset_nostr_relays(),
            AppAction::SetImageProxyEnabled { enabled } => self.set_image_proxy_enabled(enabled),
            AppAction::SetImageProxyUrl { url } => self.set_image_proxy_url(&url),
            AppAction::SetImageProxyKeyHex { key_hex } => self.set_image_proxy_key_hex(&key_hex),
            AppAction::SetImageProxySaltHex { salt_hex } => {
                self.set_image_proxy_salt_hex(&salt_hex)
            }
            AppAction::ResetImageProxySettings => self.reset_image_proxy_settings(),
            AppAction::MarkMessagesSeen {
                chat_id,
                message_ids,
            } => self.mark_messages_seen(&chat_id, &message_ids),
            AppAction::DeleteLocalMessage {
                chat_id,
                message_id,
            } => self.delete_local_message(&chat_id, &message_id),
            AppAction::UpdateGroupName { group_id, name } => {
                self.update_group_name(&group_id, &name)
            }
            AppAction::AddGroupMembers {
                group_id,
                member_inputs,
            } => self.add_group_members(&group_id, &member_inputs),
            AppAction::RemoveGroupMember {
                group_id,
                owner_pubkey_hex,
            } => self.remove_group_member(&group_id, &owner_pubkey_hex),
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
                self.handle_relay_event(event);
            }
            InternalEvent::RetryPendingOutbound => {
                let now = unix_now();
                self.retry_pending_outbound(now);
                self.retry_pending_group_controls(now);
                self.rebuild_state();
                self.persist_best_effort();
                self.emit_state();
            }
            InternalEvent::FetchTrackedPeerCatchUp => {
                let now = unix_now();
                self.push_debug_log("protocol.catch_up.schedule", "fetch tracked peers");
                self.fetch_recent_protocol_state();
                self.fetch_recent_messages_for_tracked_peers(now);
                if self.is_device_roster_open() {
                    self.fetch_pending_device_invites_for_local_owner();
                }
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
            InternalEvent::FetchCatchUpEvents(events) => {
                for event in events {
                    self.handle_relay_event(event);
                }
            }
            InternalEvent::FetchPendingDeviceInvites(events) => {
                self.handle_pending_device_invite_events(events);
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
            InternalEvent::PublishFinished {
                message_id,
                chat_id,
                success,
            } => {
                if success {
                    self.pending_outbound
                        .retain(|pending| pending.message_id != message_id);
                    self.update_message_delivery(&chat_id, &message_id, DeliveryState::Sent);
                } else if let Some(pending) = self
                    .pending_outbound
                    .iter_mut()
                    .find(|pending| pending.message_id == message_id)
                {
                    pending.in_flight = false;
                    pending.reason = PendingSendReason::PublishRetry;
                    let retry_after_secs = retry_delay_for_publish_mode(&pending.publish_mode);
                    pending.next_retry_at_secs = unix_now().get().saturating_add(retry_after_secs);
                    self.update_message_delivery(&chat_id, &message_id, DeliveryState::Queued);
                    self.schedule_pending_outbound_retry(Duration::from_secs(retry_after_secs));
                }
                self.schedule_next_pending_retry(unix_now().get());
                self.rebuild_state();
                self.persist_best_effort();
                self.emit_state();
            }
            InternalEvent::AttachmentUploadFinished { chat_id, result } => {
                self.handle_attachment_upload_finished(chat_id, result);
            }
            InternalEvent::GroupControlPublishFinished {
                operation_id,
                success,
            } => {
                if success {
                    self.pending_group_controls
                        .retain(|pending| pending.operation_id != operation_id);
                } else if let Some(pending) = self
                    .pending_group_controls
                    .iter_mut()
                    .find(|pending| pending.operation_id == operation_id)
                {
                    pending.in_flight = false;
                    pending.reason = PendingSendReason::PublishRetry;
                    pending.next_retry_at_secs =
                        unix_now().get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    self.schedule_pending_outbound_retry(Duration::from_secs(
                        PENDING_RETRY_DELAY_SECS,
                    ));
                }
                self.schedule_next_pending_retry(unix_now().get());
                self.rebuild_state();
                self.persist_best_effort();
                self.emit_state();
            }
            InternalEvent::ProtocolSubscriptionRefreshCompleted {
                token,
                applied,
                plan,
            } => {
                if token != self.protocol_subscription_runtime.refresh_token {
                    return;
                }
                self.protocol_subscription_runtime.refresh_in_flight = false;
                self.protocol_subscription_runtime.applying_plan = None;
                if applied {
                    self.push_debug_log(
                        "protocol.subscription.applied",
                        summarize_protocol_plan(plan.as_ref()),
                    );
                    self.protocol_subscription_runtime.current_plan = plan;
                    self.fetch_recent_protocol_state();
                    self.persist_best_effort();
                } else {
                    self.push_debug_log("protocol.subscription.failed", "apply returned false");
                }
                if self.protocol_subscription_runtime.refresh_dirty {
                    let force_refresh = self.protocol_subscription_runtime.force_refresh_dirty;
                    self.protocol_subscription_runtime.refresh_dirty = false;
                    self.protocol_subscription_runtime.force_refresh_dirty = false;
                    self.request_protocol_subscription_refresh_inner(force_refresh);
                }
            }
            InternalEvent::SyncComplete => {
                self.state.busy.syncing_network = false;
                self.rebuild_state();
                self.emit_state();
            }
        }
    }
}
