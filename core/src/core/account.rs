use super::account_app_keys::{
    known_app_keys_to_ndr, next_app_keys_created_at, next_removed_app_keys_created_at,
    normalize_device_label,
};
use super::invites::{load_pending_private_invite_responses, load_private_chat_invites};
use super::persistence::apply_persisted_preferences;
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

impl AppCore {
    pub(super) fn create_account(&mut self, name: &str) {
        self.state.busy.creating_account = true;
        self.emit_state();

        let owner_keys = Keys::generate();
        let device_keys = Keys::generate();
        let owner_hex = owner_keys.public_key().to_hex();
        let trimmed_name = name.trim().to_string();

        if let Err(error) = self.start_primary_session(owner_keys, device_keys, false, false) {
            self.state.toast = Some(error.to_string());
        } else {
            let profile_name = if trimmed_name.is_empty() {
                super::profile::fallback_profile_name_for_identity(&owner_hex)
            } else {
                trimmed_name
            };
            self.set_local_profile_name(&profile_name);
            self.republish_local_identity_artifacts();
        }

        self.state.busy.creating_account = false;
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn handle_app_foregrounded(&mut self) {
        // Lift the suspend gate even when not logged in so a re-foregrounded
        // unauthenticated app can process events again.
        self.suspended = false;
        if self.logged_in.is_none() {
            return;
        }

        let now = unix_now();
        let expired = self.prune_expired_messages(now.get());
        if expired > 0 {
            self.push_debug_log("messages.expired", format!("removed={expired}"));
        }
        self.push_debug_log("app.foreground", "refresh relay session");
        self.reconcile_device_sync();
        self.schedule_session_connect();
        self.request_protocol_subscription_refresh_forced_reconnect_if_offline();
        let _fetching_recent_protocol_state = self.fetch_recent_protocol_state();
        self.fetch_recent_messages_for_tracked_peers();
        self.retry_protocol_engine_pending_work("app_foreground");
        self.retry_pending_relay_publishes("app_foreground");
        self.prune_pending_private_invite_responses();
        self.refresh_protocol_sync_busy();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_current_device_labels(&mut self, device_label: &str, client_label: &str) {
        let labels = CurrentDeviceLabels {
            device_label: normalize_device_label(device_label),
            client_label: normalize_device_label(client_label),
        };
        if labels.device_label.is_none() && labels.client_label.is_none() {
            return;
        }
        self.current_device_labels = Some(labels);

        let changed =
            self.apply_current_device_labels_to_local_app_keys(!self.defer_owner_app_keys_publish);
        if changed && !self.defer_owner_app_keys_publish {
            self.publish_local_app_keys();
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn restore_primary_session(&mut self, owner_nsec: &str) {
        self.state.busy.restoring_session = true;
        self.emit_state();

        match Keys::parse(owner_nsec.trim()) {
            Ok(owner_keys) => {
                if let Err(error) =
                    self.start_primary_session(owner_keys, Keys::generate(), true, false)
                {
                    self.state.toast = Some(error.to_string());
                }
            }
            Err(_) => {
                self.state.toast = Some("Invalid key.".to_string());
            }
        }

        self.state.busy.restoring_session = false;
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn restore_account_bundle(
        &mut self,
        owner_nsec: Option<String>,
        owner_pubkey_hex: &str,
        device_nsec: &str,
    ) {
        self.push_debug_log(
            "session.restore_bundle",
            format!(
                "owner_pubkey_hex={} has_owner_nsec={}",
                owner_pubkey_hex.trim(),
                owner_nsec
                    .as_ref()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false),
            ),
        );
        self.state.busy.restoring_session = true;
        self.emit_state();

        let result = (|| -> anyhow::Result<()> {
            let owner_pubkey = parse_owner_input(owner_pubkey_hex)?;
            let owner_keys = match owner_nsec {
                Some(secret) => {
                    let keys =
                        Keys::parse(secret.trim()).map_err(|_| anyhow::anyhow!("Invalid key."))?;
                    if keys.public_key() != owner_pubkey {
                        return Err(anyhow::anyhow!(
                            "stored owner secret does not match stored owner pubkey"
                        ));
                    }
                    Some(keys)
                }
                None => None,
            };
            let device_keys = Keys::parse(device_nsec.trim())
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let allow_protocol_restore = self.restored_bundle_has_current_owner_app_keys(
                owner_pubkey,
                device_keys.public_key(),
                owner_keys.is_some(),
            );
            self.start_session(
                owner_pubkey,
                owner_keys,
                device_keys,
                true,
                allow_protocol_restore,
            )
        })();

        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        }

        self.state.busy.restoring_session = false;
        self.rebuild_state();
        self.emit_state();
    }

    fn restored_bundle_has_current_owner_app_keys(
        &mut self,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        has_owner_keys: bool,
    ) -> bool {
        if !has_owner_keys {
            return true;
        }

        let owner_hex = owner_pubkey.to_hex();
        let device_hex = device_pubkey.to_hex();
        match self.app_store.has_app_key_device(&owner_hex, &device_hex) {
            Ok(found) => found,
            Err(error) => {
                self.push_debug_log(
                    "session.restore_bundle",
                    format!("ignored_app_keys_probe_error={error}"),
                );
                false
            }
        }
    }

    pub(super) fn start_linked_device(&mut self, _owner_input: &str) {
        self.push_debug_log("session.start_linked", "create device approval request");
        self.state.busy.linking_device = true;
        self.emit_state();

        let result = self.create_pending_linked_device();
        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        }

        self.state.busy.linking_device = false;
        self.screen_stack = vec![Screen::AddDevice];
        self.rebuild_state();
        self.emit_state();
    }

    fn create_pending_linked_device(&mut self) -> anyhow::Result<()> {
        self.stop_pending_linked_device();

        if self.device_approval_relay_urls.len() != 1 {
            anyhow::bail!("Device approval requires exactly one approval relay.");
        }
        let approval_relay_urls = self.device_approval_relay_urls.clone();
        let device_keys = Keys::generate();
        let device_pubkey = device_keys.public_key();
        let current_device_labels = self.current_device_labels.clone();
        let local_request = create_nostr_identity_device_approval_request(
            &device_keys,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: None,
                request_secret: None,
                requested_at: i64::try_from(unix_now().get()).unwrap_or(i64::MAX),
                request_type: Some("device_link".to_string()),
                resources: Vec::new(),
                expires_at: None,
                profile_id: None,
                admin_app_key_pubkey: None,
                label: current_device_labels
                    .as_ref()
                    .and_then(|labels| labels.device_label.as_deref())
                    .and_then(device_approval_bootstrap_label),
            },
        )?;
        let request_keys = local_request.request_keys.clone();
        let invite =
            device_approval_pairing_invite(device_pubkey, &local_request.request.request_secret)?;
        let bootstrap = nostr_identity_device_approval_bootstrap(&local_request.request)?;
        let url = encode_nostr_identity_device_approval_bootstrap(&bootstrap, None)?;

        let client = Client::new(device_keys.clone());
        self.start_notifications_loop(client.clone());

        let invite_response_filter = Filter::new()
            .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
            .pubkeys(vec![invite.inviter_ephemeral_public_key.to_nostr()?]);
        let app_keys_authorization_filter =
            build_app_keys_device_authorization_filter(device_pubkey);
        let approval_receipt_filter = Filter::new()
            .kind(Kind::from(FACT_OP_KIND))
            .pubkey(request_keys.public_key());
        let client_for_subscription = client.clone();
        let relay_urls_for_subscription = approval_relay_urls;
        self.runtime.spawn(async move {
            ensure_session_relays_configured(
                &client_for_subscription,
                &relay_urls_for_subscription,
            )
            .await;
            connect_client_with_timeout(
                &client_for_subscription,
                Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS),
            )
            .await;
            let _ = client_for_subscription
                .subscribe_with_id(
                    SubscriptionId::new("link-device-response"),
                    invite_response_filter,
                    None,
                )
                .await;
            let _ = client_for_subscription
                .subscribe_with_id(
                    SubscriptionId::new("link-device-approval"),
                    app_keys_authorization_filter,
                    None,
                )
                .await;
            let _ = client_for_subscription
                .subscribe_with_id(
                    SubscriptionId::new("link-device-approval-receipt"),
                    approval_receipt_filter,
                    None,
                )
                .await;
        });

        self.pending_linked_device = Some(PendingLinkedDeviceState {
            device_keys,
            request_keys,
            approval_bootstrap: bootstrap,
            pairing_client: client,
            pairing_invite: invite,
            pairing_url: url,
            authorized_owner_pubkey: None,
            approval_receipt_event: None,
            authorized_app_keys_event: None,
            pending_response: None,
        });
        Ok(())
    }

    pub(super) fn stop_pending_linked_device(&mut self) {
        let Some(pending) = self.pending_linked_device.take() else {
            return;
        };
        let client = pending.pairing_client;
        self.runtime.spawn(async move {
            client.unsubscribe_all().await;
            let _ = client.shutdown().await;
        });
    }

    pub(super) fn complete_pending_linked_device(
        &mut self,
        owner_pubkey: PublicKey,
        peer_device_id: String,
        session_state: SessionState,
        device_keys: Keys,
    ) -> anyhow::Result<()> {
        self.stop_pending_linked_device();
        self.start_session(owner_pubkey, None, device_keys, false, false)?;
        let retry_batch = self
            .protocol_engine
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Link failed."))?
            .import_session_state(
                owner_pubkey,
                Some(peer_device_id),
                session_state,
                unix_now(),
            )?;
        self.mark_mobile_push_dirty();
        self.process_protocol_engine_retry_batch("linked_device_import", retry_batch);
        self.request_protocol_subscription_refresh_forced();
        self.fetch_recent_protocol_state();
        self.persist_best_effort();
        self.rebuild_state();
        self.emit_state();
        Ok(())
    }

    pub(super) fn logout(&mut self) {
        self.push_debug_log("session.logout", "clearing runtime state");
        let previous_rev = self.state.rev;
        self.stop_pending_linked_device();
        self.stop_device_sync();
        self.reset_pending_invite_acceptance();
        self.private_chat_invites.clear();
        self.pending_private_invite_responses.clear();
        self.pending_private_invite_cleanup_retry = false;
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.wrapping_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        self.protocol_engine = None;
        if let Some(logged_in) = self.logged_in.take() {
            let client = logged_in.client.clone();
            self.runtime.spawn(async move {
                client.unsubscribe_all().await;
                let _ = client.shutdown().await;
            });
        }

        self.threads.clear();
        self.active_chat_id = None;
        self.screen_stack.clear();
        self.owner_profiles.clear();
        self.profile_metadata_fetch_inflight.clear();
        self.app_keys.clear();
        self.groups.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.typing_floor_secs.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_transport_runtime = RelayTransportRuntime::default();
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.relay_connected_count = 0;
        self.all_relays_offline_since_secs = None;
        self.debug_snapshot_write_generation = self.debug_snapshot_write_generation.wrapping_add(1);
        self.debug_snapshot_write_inflight = false;
        self.debug_snapshot_write_dirty = false;
        self.cached_mobile_push = MobilePushSyncSnapshot::default();
        self.mobile_push_dirty = true;
        self.last_emitted_state = None;
        self.next_message_id = 1;
        self.state = AppState::empty();
        self.state.rev = previous_rev;
        self.clear_persistence_best_effort();
        self.emit_state();
    }

    pub(super) fn remove_authorized_device(&mut self, device_pubkey_hex: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore a profile first.".to_string());
            self.emit_state();
            return;
        };
        if logged_in.owner_keys.is_none() {
            self.state.toast = Some("Only the primary device can manage devices.".to_string());
            self.emit_state();
            return;
        }

        let Ok(device_pubkey) = parse_device_input(device_pubkey_hex) else {
            self.state.toast = Some("Invalid device key.".to_string());
            self.emit_state();
            return;
        };
        if device_pubkey == logged_in.device_keys.public_key() {
            self.state.toast = Some("The current device cannot remove itself.".to_string());
            self.emit_state();
            return;
        }

        self.remove_local_app_key_device(logged_in.owner_pubkey, device_pubkey);
        self.publish_local_app_keys();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn acknowledge_revoked_device(&mut self) {
        if matches!(
            self.logged_in
                .as_ref()
                .map(|logged_in| logged_in.authorization_state),
            Some(LocalAuthorizationState::Revoked)
        ) {
            self.screen_stack.clear();
            self.rebuild_state();
            self.emit_state();
        }
    }

    pub(super) fn start_primary_session(
        &mut self,
        owner_keys: Keys,
        device_keys: Keys,
        allow_restore: bool,
        allow_protocol_restore: bool,
    ) -> anyhow::Result<()> {
        let owner_pubkey = owner_keys.public_key();
        self.push_debug_log(
            "session.start_primary",
            format!(
                "owner_pubkey={} allow_restore={} allow_protocol_restore={}",
                owner_pubkey.to_hex(),
                allow_restore,
                allow_protocol_restore,
            ),
        );
        self.start_session(
            owner_pubkey,
            Some(owner_keys),
            device_keys,
            allow_restore,
            allow_protocol_restore,
        )
    }

    pub(super) fn start_session(
        &mut self,
        owner_pubkey: OwnerPubkey,
        owner_keys: Option<Keys>,
        device_keys: Keys,
        allow_restore: bool,
        allow_protocol_restore: bool,
    ) -> anyhow::Result<()> {
        self.push_debug_log(
            "session.start",
            format!(
                "owner={} has_owner_keys={} allow_restore={} allow_protocol_restore={}",
                owner_pubkey.to_hex(),
                owner_keys.is_some(),
                allow_restore,
                allow_protocol_restore,
            ),
        );
        self.stop_pending_linked_device();
        self.stop_device_sync();
        self.reset_pending_invite_acceptance();
        self.pending_private_invite_responses.clear();
        self.pending_private_invite_cleanup_retry = false;
        self.protocol_engine = None;
        if let Some(existing) = self.logged_in.take() {
            let client = existing.client;
            self.runtime.spawn(async move {
                client.unsubscribe_all().await;
                let _ = client.shutdown().await;
            });
        }

        self.threads.clear();
        self.active_chat_id = None;
        self.screen_stack.clear();
        self.owner_profiles.clear();
        self.profile_metadata_fetch_inflight.clear();
        self.app_keys.clear();
        self.groups.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.typing_floor_secs.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_transport_runtime = RelayTransportRuntime::default();
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.relay_connected_count = 0;
        self.all_relays_offline_since_secs = None;
        self.defer_owner_app_keys_publish = false;
        self.pending_relay_publishes.clear();
        self.pending_relay_publish_inflight.clear();
        self.debug_log.clear();
        self.debug_event_counters = DebugEventCounters::default();
        self.debug_snapshot_write_generation = self.debug_snapshot_write_generation.wrapping_add(1);
        self.debug_snapshot_write_inflight = false;
        self.debug_snapshot_write_dirty = false;
        self.next_message_id = 1;

        let persisted = if allow_restore {
            match self.load_persisted() {
                Ok(persisted) => persisted,
                Err(error) => {
                    self.push_debug_log(
                        "session.restore_state",
                        format!("ignored_invalid_persistence={error}"),
                    );
                    None
                }
            }
        } else {
            None
        };
        self.push_debug_log(
            "session.restore_state",
            format!("persisted_present={}", persisted.is_some()),
        );
        if let Some(persisted) = &persisted {
            self.active_chat_id = persisted.active_chat_id.clone();
            self.next_message_id = persisted.next_message_id.max(1);
            self.owner_profiles = persisted.owner_profiles.clone();
            self.chat_message_ttl_seconds = persisted.chat_message_ttl_seconds.clone();
            apply_persisted_preferences(&mut self.preferences, &persisted.preferences);
            self.seen_event_order = persisted
                .seen_event_ids
                .iter()
                .rev()
                .take(MAX_SEEN_EVENT_IDS)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            self.seen_event_ids = self.seen_event_order.iter().cloned().collect();
            self.app_keys = persisted
                .app_keys
                .iter()
                .cloned()
                .map(|entry| (entry.owner_pubkey_hex.clone(), entry))
                .collect();
            self.groups = persisted
                .groups
                .iter()
                .cloned()
                .map(|group| (group.group_id.clone(), group))
                .collect();
            self.group_pictures = persisted.group_pictures.clone();
            self.threads = persisted
                .threads
                .iter()
                .map(|thread| {
                    let updated_at_secs = thread.updated_at_secs.max(
                        thread
                            .messages
                            .iter()
                            .map(|message| message.created_at_secs)
                            .max()
                            .unwrap_or(0),
                    );
                    (
                        thread.chat_id.clone(),
                        ThreadRecord {
                            chat_id: thread.chat_id.clone(),
                            unread_count: thread.unread_count,
                            updated_at_secs,
                            messages: thread
                                .messages
                                .iter()
                                .map(|message| {
                                    let (body, parsed_attachments) =
                                        extract_message_attachments(&message.body);
                                    ChatMessageSnapshot {
                                        id: message.id.clone(),
                                        chat_id: message.chat_id.clone(),
                                        kind: message.kind.clone(),
                                        author: message.author.clone(),
                                        author_owner_pubkey_hex: message
                                            .author_owner_pubkey_hex
                                            .clone(),
                                        author_picture_url: None,
                                        body,
                                        attachments: if message.attachments.is_empty() {
                                            parsed_attachments
                                        } else {
                                            message.attachments.clone()
                                        },
                                        reactions: message.reactions.clone(),
                                        reactors: message.reactors.clone(),
                                        is_outgoing: message.is_outgoing,
                                        created_at_secs: message.created_at_secs,
                                        expires_at_secs: message.expires_at_secs,
                                        delivery: message.delivery.clone().into(),
                                        recipient_deliveries: message.recipient_deliveries.clone(),
                                        delivery_trace: message.delivery_trace.clone(),
                                        source_event_id: message.source_event_id.clone(),
                                    }
                                })
                                .collect(),
                            draft: thread.draft.clone(),
                        },
                    )
                })
                .collect();
            // Prime the typing floor from the most recent message we
            // restored for each chat. Without this, a typing rumor
            // delivered right after restart could re-arm an indicator
            // even though we already have a newer message on disk.
            self.typing_floor_secs = self
                .threads
                .iter()
                .filter_map(|(chat_id, thread)| {
                    thread
                        .messages
                        .last()
                        .map(|message| (chat_id.clone(), message.created_at_secs))
                })
                .collect();
        }
        let previous_authorization_state = persisted
            .as_ref()
            .and_then(|state| state.authorization_state.clone())
            .map(LocalAuthorizationState::from);

        let device_pubkey = device_keys.public_key();
        let should_defer_owner_app_keys_publish =
            owner_keys.is_some() && allow_restore && !allow_protocol_restore;
        if owner_keys.is_some() && !should_defer_owner_app_keys_publish {
            let current_device_labels = self.current_device_labels.clone();
            self.upsert_local_app_key_device_with_labels(
                owner_pubkey,
                device_pubkey,
                current_device_labels.as_ref(),
                true,
            );
        }
        self.defer_owner_app_keys_publish = should_defer_owner_app_keys_publish;

        let storage = Arc::new(SqliteStorageAdapter::new(
            self.app_store.shared(),
            owner_pubkey.to_hex(),
            device_pubkey.to_hex(),
        )) as Arc<dyn StorageAdapter>;
        self.private_chat_invites = load_private_chat_invites(storage.as_ref())?;
        self.pending_private_invite_responses =
            load_pending_private_invite_responses(storage.as_ref())?;

        let protocol_engine =
            ProtocolEngine::load_or_create_for_local_device(storage, owner_pubkey, &device_keys)?;
        self.protocol_engine = Some(protocol_engine);

        let authorization_state = self.restored_local_authorization_state(
            owner_keys.as_ref(),
            owner_pubkey,
            device_pubkey,
            previous_authorization_state,
        );
        if let Some(chat_id) = self.active_chat_id.clone() {
            self.screen_stack = vec![Screen::Chat { chat_id }];
        }

        let client = Client::new(device_keys.clone());
        let relay_urls = relay_urls_from_strings(&self.preferences.nostr_relay_urls);
        self.start_notifications_loop(client.clone());

        self.logged_in = Some(LoggedInState {
            owner_pubkey,
            owner_keys: owner_keys.clone(),
            device_keys: device_keys.clone(),
            client,
            relay_urls,
            authorization_state,
        });
        self.prune_orphaned_pending_private_invite_responses();
        self.refresh_local_authorization_state();
        self.reconcile_device_sync();
        self.push_debug_log(
            "session.authorization",
            format!(
                "state={authorization_state:?} owner={} device={}",
                owner_pubkey.to_hex(),
                device_pubkey.to_hex()
            ),
        );
        self.emit_account_bundle_update(owner_keys.as_ref(), &device_keys);
        if allow_restore {
            self.rebuild_state();
            self.emit_state();
            if !self.queue_session_startup_follow_up() {
                self.run_session_startup_follow_up();
            }
        } else {
            self.run_session_startup_follow_up();
        }
        Ok(())
    }

    fn queue_session_startup_follow_up(&self) -> bool {
        self.core_sender
            .send(CoreMsg::Internal(Box::new(
                InternalEvent::SessionStartupFollowUp,
            )))
            .is_ok()
    }

    pub(super) fn run_session_startup_follow_up(&mut self) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return;
        };
        let owner_pubkey = logged_in.owner_pubkey;

        self.ingest_restored_app_keys_for_protocol();
        self.retry_all_pending_private_invite_responses();
        self.load_pending_relay_publish_queue(owner_pubkey);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        self.start_relay_status_watchers();
        self.schedule_session_connect();
        self.republish_local_identity_artifacts();
        self.drain_pending_mobile_push_events();
        self.retry_protocol_engine_pending_work("session_start");
        self.retry_pending_relay_publishes("session_start");
        self.schedule_next_message_expiry();
        self.request_protocol_subscription_refresh();
        self.fetch_recent_protocol_state();
        self.refresh_protocol_sync_busy();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
    }

    fn ingest_restored_app_keys_for_protocol(&mut self) {
        let existing_app_keys = self.app_keys.values().cloned().collect::<Vec<_>>();
        let mut app_keys_retry_batch = ProtocolRetryBatch::default();
        for app_keys in existing_app_keys {
            if let (Ok(owner), Some(keys)) = (
                PublicKey::parse(&app_keys.owner_pubkey_hex),
                known_app_keys_to_ndr(&app_keys),
            ) {
                if let Some(protocol_engine) = self.protocol_engine.as_mut() {
                    if let Ok(batch) = protocol_engine.ingest_app_keys_snapshot(
                        owner,
                        keys,
                        app_keys.created_at_secs,
                    ) {
                        Self::append_protocol_retry_batch(&mut app_keys_retry_batch, batch);
                    }
                }
            }
        }
        self.process_protocol_engine_retry_batch("session_start_app_keys", app_keys_retry_batch);
    }

    fn load_pending_relay_publish_queue(&mut self, owner_pubkey: PublicKey) {
        let owner_pubkey_hex = owner_pubkey.to_hex();
        match self
            .app_store
            .prune_superseded_protocol_invite_response_publishes(&owner_pubkey_hex)
        {
            Ok(pruned) if pruned > 0 => {
                self.push_debug_log(
                    "publish.runtime.queue",
                    format!("pruned_superseded_protocol_bootstrap={pruned}"),
                );
            }
            Err(error) => {
                self.push_debug_log(
                    "publish.runtime.queue",
                    format!("bootstrap_prune_failed={error}"),
                );
            }
            _ => {}
        }
        match self
            .app_store
            .prune_pending_relay_control_publishes_to_limit(
                &owner_pubkey_hex,
                PENDING_RELAY_CONTROL_PUBLISH_MAX_ROWS,
            ) {
            Ok(pruned) if pruned > 0 => {
                self.push_debug_log(
                    "publish.runtime.queue",
                    format!(
                        "pruned_control_backlog={pruned} max={PENDING_RELAY_CONTROL_PUBLISH_MAX_ROWS}"
                    ),
                );
            }
            Err(error) => {
                self.push_debug_log(
                    "publish.runtime.queue",
                    format!("control_backlog_prune_failed={error}"),
                );
            }
            _ => {}
        }
        match self
            .app_store
            .load_pending_relay_publishes(&owner_pubkey_hex)
        {
            Ok(pending) => {
                self.pending_relay_publishes = pending
                    .into_iter()
                    .map(|pending| (pending.event_id.clone(), pending))
                    .collect();
                let restored_messages = self
                    .threads
                    .iter()
                    .flat_map(|(chat_id, thread)| {
                        thread
                            .messages
                            .iter()
                            .map(|message| (chat_id.clone(), message.id.clone()))
                    })
                    .collect::<Vec<_>>();
                for (chat_id, message_id) in restored_messages {
                    self.sync_message_delivery_trace(&chat_id, &message_id);
                }
            }
            Err(error) => {
                self.push_debug_log("publish.runtime.queue", format!("load_failed={error}"));
            }
        }
    }

    #[cfg(test)]
    pub(super) fn upsert_local_app_key_device(
        &mut self,
        owner: PublicKey,
        device: PublicKey,
    ) -> bool {
        self.upsert_local_app_key_device_with_labels(owner, device, None, true)
    }

    pub(super) fn upsert_local_app_key_device_with_labels(
        &mut self,
        owner: PublicKey,
        device: PublicKey,
        labels: Option<&CurrentDeviceLabels>,
        create_if_missing: bool,
    ) -> bool {
        let owner_hex = owner.to_hex();
        let now = unix_now().get();
        if !self.app_keys.contains_key(&owner_hex) && !create_if_missing {
            return false;
        }
        let entry = self
            .app_keys
            .entry(owner_hex.clone())
            .or_insert_with(|| KnownAppKeys {
                owner_pubkey_hex: owner_hex,
                created_at_secs: now,
                devices: Vec::new(),
            });
        let device_hex = device.to_hex();
        if let Some(existing) = entry
            .devices
            .iter()
            .position(|existing| existing.identity_pubkey_hex == device_hex)
        {
            let Some(labels) = labels else {
                return false;
            };
            let Some(existing) = entry.devices.get_mut(existing) else {
                return false;
            };
            if existing.device_label == labels.device_label
                && existing.client_label == labels.client_label
            {
                return false;
            }
            let next_created_at = next_app_keys_created_at(now, entry.created_at_secs);
            existing.device_label = labels.device_label.clone();
            existing.client_label = labels.client_label.clone();
            existing.label_updated_at_secs = next_created_at;
            entry.created_at_secs = next_created_at;
            return true;
        }
        let next_created_at = next_app_keys_created_at(now, entry.created_at_secs);
        entry.devices.push(KnownAppKeyDevice {
            identity_pubkey_hex: device_hex,
            created_at_secs: next_created_at,
            device_label: labels.and_then(|labels| labels.device_label.clone()),
            client_label: labels.and_then(|labels| labels.client_label.clone()),
            label_updated_at_secs: labels.map(|_| next_created_at).unwrap_or_default(),
        });
        entry.created_at_secs = next_created_at;
        entry
            .devices
            .sort_by(|left, right| left.identity_pubkey_hex.cmp(&right.identity_pubkey_hex));
        true
    }

    fn apply_current_device_labels_to_local_app_keys(&mut self, create_if_missing: bool) -> bool {
        let Some(labels) = self.current_device_labels.clone() else {
            return false;
        };
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        if logged_in.owner_keys.is_none() {
            return false;
        }
        self.upsert_local_app_key_device_with_labels(
            logged_in.owner_pubkey,
            logged_in.device_keys.public_key(),
            Some(&labels),
            create_if_missing,
        )
    }

    pub(super) fn apply_current_device_labels_to_known_app_keys(
        &self,
        known: &mut KnownAppKeys,
        device_pubkey: PublicKey,
    ) -> bool {
        let Some(labels) = self.current_device_labels.as_ref() else {
            return false;
        };
        let device_hex = device_pubkey.to_hex();
        let Some(device) = known
            .devices
            .iter_mut()
            .find(|device| device.identity_pubkey_hex == device_hex)
        else {
            return false;
        };
        if device.device_label == labels.device_label && device.client_label == labels.client_label
        {
            return false;
        }
        let next_created_at = next_app_keys_created_at(unix_now().get(), known.created_at_secs);
        device.device_label = labels.device_label.clone();
        device.client_label = labels.client_label.clone();
        device.label_updated_at_secs = next_created_at;
        known.created_at_secs = next_created_at;
        true
    }

    pub(super) fn remove_local_app_key_device(&mut self, owner: PublicKey, device: PublicKey) {
        if let Some(entry) = self.app_keys.get_mut(&owner.to_hex()) {
            let latest_device_created_at = entry
                .devices
                .iter()
                .map(|candidate| {
                    candidate
                        .created_at_secs
                        .max(candidate.label_updated_at_secs)
                })
                .max()
                .unwrap_or_default();
            entry
                .devices
                .retain(|candidate| candidate.identity_pubkey_hex != device.to_hex());
            entry.created_at_secs = next_removed_app_keys_created_at(
                unix_now().get(),
                entry.created_at_secs,
                latest_device_created_at,
            );
        }
    }

    pub(super) fn refresh_local_authorization_state(&mut self) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        let previous = logged_in.authorization_state;
        let next = self.local_authorization_state(
            logged_in.owner_keys.as_ref(),
            logged_in.owner_pubkey,
            logged_in.device_keys.public_key(),
            Some(previous),
        );
        if next == previous {
            return false;
        }

        let owner_hex = logged_in.owner_pubkey.to_hex();
        let device_hex = logged_in.device_keys.public_key().to_hex();
        if let Some(logged_in) = self.logged_in.as_mut() {
            logged_in.authorization_state = next;
        }
        self.push_debug_log(
            "session.authorization",
            format!("state={next:?} owner={owner_hex} device={device_hex}"),
        );
        true
    }

    pub(super) fn restored_local_authorization_state(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
    ) -> LocalAuthorizationState {
        self.local_authorization_state_inner(
            owner_keys,
            owner_pubkey,
            device_pubkey,
            previous,
            false,
        )
    }

    pub(super) fn local_authorization_state(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
    ) -> LocalAuthorizationState {
        self.local_authorization_state_inner(
            owner_keys,
            owner_pubkey,
            device_pubkey,
            previous,
            true,
        )
    }

    fn local_authorization_state_inner(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
        allow_revoke: bool,
    ) -> LocalAuthorizationState {
        if owner_keys.is_some() {
            return LocalAuthorizationState::Authorized;
        }

        let owner_hex = owner_pubkey.to_hex();
        let device_hex = device_pubkey.to_hex();
        let Some(app_keys) = self.app_keys.get(&owner_hex) else {
            return previous.unwrap_or(LocalAuthorizationState::AwaitingApproval);
        };

        let registered = app_keys
            .devices
            .iter()
            .any(|device| device.identity_pubkey_hex.eq_ignore_ascii_case(&device_hex));
        if registered {
            let has_local_session = self.protocol_engine.as_ref().is_some_and(|engine| {
                // The owner-signed AppKeys snapshot above already proves
                // that this local device is authorized. Here a session is
                // only an approval-handshake readiness signal; its remote
                // device must not be promoted to the owner for messaging
                // unless that separate O -> D binding is verified.
                ProtocolEngine::active_session_count_for_owner_with_snapshot(
                    &engine.session_manager_snapshot(),
                    owner_pubkey,
                ) > 0
            });
            if has_local_session {
                return LocalAuthorizationState::Authorized;
            }
            return LocalAuthorizationState::AwaitingApproval;
        }

        if !allow_revoke && previous == Some(LocalAuthorizationState::Authorized) {
            return LocalAuthorizationState::Authorized;
        }

        match previous {
            Some(LocalAuthorizationState::Authorized) | Some(LocalAuthorizationState::Revoked) => {
                LocalAuthorizationState::Revoked
            }
            _ => LocalAuthorizationState::AwaitingApproval,
        }
    }
}

fn device_approval_bootstrap_label(label: &str) -> Option<String> {
    let label = normalize_device_label(label)?;
    let mut end = label
        .len()
        .min(NOSTR_IDENTITY_DEVICE_APPROVAL_LABEL_MAX_BYTES);
    while !label.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let bounded = label[..end].trim_end();
    (!bounded.is_empty()).then(|| bounded.to_string())
}

pub(super) fn device_approval_pairing_invite(
    device_app_key_pubkey: PublicKey,
    request_secret: &str,
) -> anyhow::Result<Invite> {
    let secret = URL_SAFE_NO_PAD
        .decode(request_secret)
        .map_err(|error| anyhow::anyhow!("invalid device approval request secret: {error}"))?;
    if secret.len() != 32 {
        anyhow::bail!("device approval request secret must be 32 bytes");
    }
    let secret_hex = secret
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    deterministic_link_invite_for_device(device_app_key_pubkey, &secret_hex)
        .map_err(anyhow::Error::from)
}

pub(super) fn nostr_identity_profile_id_for_owner(owner_pubkey: PublicKey) -> NostrIdentityId {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(b"iris-chat-rs:nostr-identity-profile-id:v1\n");
    digest.update(owner_pubkey.to_hex().as_bytes());
    let hash = digest.finalize();
    let mut bytes = [0u8; 16];
    for (target, source) in bytes.iter_mut().zip(hash.iter()) {
        *target = *source;
    }
    NostrIdentityId::from_uuid(uuid::Uuid::from_bytes(bytes))
}
