use super::invites::{load_private_chat_invites, parse_public_invite_input};
use super::*;

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
        if self.logged_in.is_none() {
            return;
        }

        let now = unix_now();
        let expired = self.prune_expired_messages(now.get());
        if expired > 0 {
            self.push_debug_log("messages.expired", format!("removed={expired}"));
        }
        self.push_debug_log("app.foreground", "refresh relay session");
        self.schedule_session_connect();
        self.request_protocol_subscription_refresh_forced_reconnect_if_offline();
        let fetching_recent_protocol_state = self.fetch_recent_protocol_state();
        self.fetch_recent_messages_for_tracked_peers(now);
        self.retry_pending_relay_publishes("app_foreground");
        self.state.busy.syncing_network = fetching_recent_protocol_state;
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
        match self.app_store.load_state() {
            Ok(Some(persisted)) => persisted.app_keys.iter().any(|app_keys| {
                app_keys.owner_pubkey_hex.eq_ignore_ascii_case(&owner_hex)
                    && app_keys
                        .devices
                        .iter()
                        .any(|device| device.identity_pubkey_hex.eq_ignore_ascii_case(&device_hex))
            }),
            Ok(None) => false,
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
        self.push_debug_log("session.start_linked", "create ownerless link invite");
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

        let device_keys = Keys::generate();
        let device_pubkey = device_keys.public_key();
        let device_id = device_pubkey.to_hex();
        let mut invite = Invite::create_new(device_pubkey, Some(device_id), Some(1))?;
        invite.purpose = Some("link".to_string());
        let url = nostr_double_ratchet_nostr::invite_url(&invite, CHAT_INVITE_ROOT_URL)?;

        let client = Client::new(device_keys.clone());
        let relay_urls = relay_urls_from_strings(&self.preferences.nostr_relay_urls);
        self.start_notifications_loop(client.clone());

        let filter = Filter::new()
            .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
            .pubkeys(vec![invite.inviter_ephemeral_public_key.to_nostr()?]);
        let client_for_subscription = client.clone();
        let relay_urls_for_subscription = relay_urls.clone();
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
                .subscribe_with_id(SubscriptionId::new("link-device-response"), filter, None)
                .await;
        });

        self.pending_linked_device = Some(PendingLinkedDeviceState {
            device_keys,
            client,
            invite,
            url,
        });
        Ok(())
    }

    pub(super) fn stop_pending_linked_device(&mut self) {
        let Some(pending) = self.pending_linked_device.take() else {
            return;
        };
        let client = pending.client;
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
        let Some(logged_in) = self.logged_in.as_ref() else {
            return Err(anyhow::anyhow!("Link failed."));
        };
        let effects = logged_in.ndr_runtime.import_session_state(
            owner_pubkey,
            Some(peer_device_id),
            session_state,
        )?;
        self.mark_mobile_push_dirty();
        self.process_runtime_effects(effects);
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
        self.private_chat_invites.clear();
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.wrapping_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
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
        self.app_keys.clear();
        self.groups.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.typing_floor_secs.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.direct_message_subscriptions = DirectMessageSubscriptionTracker::new();
        self.relay_status_watch_urls.clear();
        self.setup_user_done.clear();
        self.cached_mobile_push = MobilePushSyncSnapshot::default();
        self.mobile_push_dirty = true;
        self.last_emitted_state = None;
        self.next_message_id = 1;
        self.state = AppState::empty();
        self.state.rev = previous_rev;
        self.clear_persistence_best_effort();
        self.emit_state();
    }

    pub(super) fn add_authorized_device(&mut self, device_input: &str) {
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

        let owner_pubkey = logged_in.owner_pubkey;
        if let Ok(invite) = parse_link_device_invite_input(device_input, owner_pubkey) {
            self.state.busy.updating_roster = true;
            self.emit_state();

            let result = self.accept_link_device_invite(invite);
            if let Err(error) = result {
                self.state.toast = Some(error.to_string());
            }

            self.state.busy.updating_roster = false;
            self.rebuild_state();
            self.persist_best_effort();
            self.emit_state();
            return;
        }

        let Ok(device_pubkey) = parse_device_input(device_input) else {
            self.state.toast = Some("Invalid device key.".to_string());
            self.emit_state();
            return;
        };

        self.upsert_local_app_key_device(owner_pubkey, device_pubkey);
        self.publish_local_app_keys();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn accept_link_device_invite(&mut self, invite: Invite) -> anyhow::Result<()> {
        let logged_in = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
        if logged_in.owner_keys.is_none() {
            return Err(anyhow::anyhow!(
                "Only the primary device can manage devices."
            ));
        }
        let owner_pubkey = logged_in.owner_pubkey;
        let device_pubkey = logged_in.device_keys.public_key();
        let (session, response) = invite.accept_with_owner(
            device_pubkey,
            logged_in.device_keys.secret_key().to_secret_bytes(),
            Some(device_pubkey.to_hex()),
            Some(owner_pubkey),
        )?;
        let effects = logged_in.ndr_runtime.import_session_state(
            owner_pubkey,
            Some(invite.inviter_device_pubkey.to_hex()),
            session.state,
        )?;
        let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)?;
        self.upsert_local_app_key_device(owner_pubkey, invite.inviter_device_pubkey.to_nostr()?);
        self.publish_local_app_keys();
        self.publish_runtime_event(response_event, "runtime", None);
        self.mark_mobile_push_dirty();
        self.process_runtime_effects(effects);
        Ok(())
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
        self.app_keys.clear();
        self.groups.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.typing_floor_secs.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.direct_message_subscriptions = DirectMessageSubscriptionTracker::new();
        self.defer_owner_app_keys_publish = false;
        self.pending_relay_publishes.clear();
        self.pending_relay_publish_inflight.clear();
        self.debug_log.clear();
        self.debug_event_counters = DebugEventCounters::default();
        self.next_message_id = 1;

        let now = unix_now();
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
            self.preferences.send_typing_indicators = persisted.preferences.send_typing_indicators;
            self.preferences.send_read_receipts = persisted.preferences.send_read_receipts;
            self.preferences.desktop_notifications_enabled =
                persisted.preferences.desktop_notifications_enabled;
            self.preferences.invite_acceptance_notifications_enabled = persisted
                .preferences
                .invite_acceptance_notifications_enabled;
            self.preferences.startup_at_login_enabled =
                persisted.preferences.startup_at_login_enabled;
            self.preferences.nearby_bluetooth_enabled =
                persisted.preferences.nearby_bluetooth_enabled;
            self.preferences.nearby_lan_enabled = persisted.preferences.nearby_lan_enabled;
            self.preferences.nostr_relay_urls =
                normalize_nostr_relay_urls(&persisted.preferences.nostr_relay_urls);
            self.preferences.image_proxy_enabled = persisted.preferences.image_proxy_enabled;
            self.preferences.image_proxy_url = persisted.preferences.image_proxy_url.clone();
            self.preferences.image_proxy_key_hex =
                persisted.preferences.image_proxy_key_hex.clone();
            self.preferences.image_proxy_salt_hex =
                persisted.preferences.image_proxy_salt_hex.clone();
            self.preferences.mobile_push_server_url =
                persisted.preferences.mobile_push_server_url.clone();
            self.preferences.muted_chat_ids = persisted.preferences.muted_chat_ids.clone();
            self.preferences.muted_chat_ids.sort();
            self.preferences.muted_chat_ids.dedup();
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
            self.upsert_local_app_key_device(owner_pubkey, device_pubkey);
        }
        self.defer_owner_app_keys_publish = should_defer_owner_app_keys_publish;

        let storage = Arc::new(SqliteStorageAdapter::new(
            self.app_store.shared(),
            owner_pubkey.to_hex(),
            device_pubkey.to_hex(),
        )) as Arc<dyn StorageAdapter>;
        self.private_chat_invites = load_private_chat_invites(storage.as_ref())?;
        match import_legacy_ndr_storage(storage.as_ref(), owner_pubkey) {
            Ok(summary) => {
                if summary.imported > 0 || summary.replaced_empty > 0 {
                    self.push_debug_log(
                        "session.legacy_ndr_import",
                        format!(
                            "imported={} replaced_empty={} skipped_existing={} skipped_invalid={}",
                            summary.imported,
                            summary.replaced_empty,
                            summary.skipped_existing,
                            summary.skipped_invalid
                        ),
                    );
                }
            }
            Err(error) => {
                self.push_debug_log("session.legacy_ndr_import", format!("ignored={error}"));
            }
        }
        let device_id = device_pubkey.to_hex();
        let mut local_invite =
            load_or_create_local_invite(storage.as_ref(), device_pubkey, &device_id, owner_pubkey)?;
        let ndr_runtime = NdrRuntime::new(
            device_pubkey,
            device_keys.secret_key().to_secret_bytes(),
            device_id,
            owner_pubkey,
            Some(storage),
            Some(local_invite.clone()),
        );
        let mut startup_effects = ndr_runtime.init()?;
        if let Some(runtime_invite) = ndr_runtime.local_invite() {
            local_invite = runtime_invite;
        }
        ndr_runtime.set_auto_adopt_chat_settings(true);

        for app_keys in self.app_keys.values() {
            if let (Ok(owner), Some(keys)) = (
                PublicKey::parse(&app_keys.owner_pubkey_hex),
                known_app_keys_to_ndr(app_keys),
            ) {
                if let Ok(effects) =
                    ndr_runtime.ingest_app_keys_snapshot(owner, keys, app_keys.created_at_secs)
                {
                    startup_effects.extend(effects);
                }
            }
        }
        ndr_runtime.sync_groups(self.groups.values().cloned().collect())?;

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
            ndr_runtime,
            local_invite,
            authorization_state,
        });
        self.process_runtime_effects(startup_effects);
        match self
            .app_store
            .load_pending_relay_publishes(&owner_pubkey.to_hex())
        {
            Ok(pending) => {
                self.pending_relay_publishes = pending
                    .into_iter()
                    .map(|pending| (pending.event_id.clone(), pending))
                    .collect();
                for pending in self
                    .pending_relay_publishes
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                {
                    if let (Some(chat_id), Some(message_id)) = (pending.chat_id, pending.message_id)
                    {
                        self.sync_message_delivery_trace(&chat_id, &message_id);
                    }
                }
            }
            Err(error) => {
                self.push_debug_log("publish.runtime.queue", format!("load_failed={error}"));
            }
        }

        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.start_relay_status_watchers();
        self.schedule_session_connect();
        self.emit_account_bundle_update(owner_keys.as_ref(), &device_keys);
        self.republish_local_identity_artifacts();
        self.drain_pending_mobile_push_events();
        self.retry_pending_relay_publishes("session_start");
        self.schedule_next_message_expiry();
        self.request_protocol_subscription_refresh();
        self.fetch_recent_protocol_state();
        self.state.busy.syncing_network = true;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
        self.push_debug_log(
            "session.authorization",
            format!(
                "state={authorization_state:?} owner={} device={}",
                owner_pubkey.to_hex(),
                device_pubkey.to_hex()
            ),
        );
        self.schedule_tracked_peer_catch_up(Duration::from_secs(RESUBSCRIBE_CATCH_UP_DELAY_SECS));
        let _ = now;
        Ok(())
    }

    pub(super) fn upsert_local_app_key_device(&mut self, owner: PublicKey, device: PublicKey) {
        let owner_hex = owner.to_hex();
        let now = unix_now().get();
        let entry = self
            .app_keys
            .entry(owner_hex.clone())
            .or_insert_with(|| KnownAppKeys {
                owner_pubkey_hex: owner_hex,
                created_at_secs: now,
                devices: Vec::new(),
            });
        let next_created_at = if now <= entry.created_at_secs {
            entry.created_at_secs.saturating_add(1)
        } else {
            now
        };
        if !entry
            .devices
            .iter()
            .any(|existing| existing.identity_pubkey_hex == device.to_hex())
        {
            entry.devices.push(KnownAppKeyDevice {
                identity_pubkey_hex: device.to_hex(),
                created_at_secs: next_created_at,
            });
        }
        entry.created_at_secs = next_created_at;
        entry
            .devices
            .sort_by(|left, right| left.identity_pubkey_hex.cmp(&right.identity_pubkey_hex));
    }

    pub(super) fn remove_local_app_key_device(&mut self, owner: PublicKey, device: PublicKey) {
        if let Some(entry) = self.app_keys.get_mut(&owner.to_hex()) {
            entry
                .devices
                .retain(|candidate| candidate.identity_pubkey_hex != device.to_hex());
            entry.created_at_secs = unix_now().get();
        }
    }

    pub(super) fn apply_known_app_keys_snapshot(
        &mut self,
        owner: PublicKey,
        incoming_app_keys: &AppKeys,
        incoming_created_at: u64,
    ) -> Option<(AppKeys, u64)> {
        let owner_hex = owner.to_hex();
        let current = self.app_keys.get(&owner_hex).cloned();
        let current_app_keys = current.as_ref().and_then(known_app_keys_to_ndr);
        let current_created_at = current
            .as_ref()
            .map(|known| known.created_at_secs)
            .unwrap_or_default();
        let required_device = self
            .logged_in
            .as_ref()
            .filter(|logged_in| {
                self.defer_owner_app_keys_publish
                    && logged_in.owner_keys.is_some()
                    && logged_in.owner_pubkey == owner
            })
            .map(|logged_in| {
                DeviceEntry::new(logged_in.device_keys.public_key(), unix_now().get())
            });
        let applied = apply_app_keys_snapshot_with_required_device(
            current_app_keys.as_ref(),
            current_created_at,
            incoming_app_keys,
            incoming_created_at,
            required_device,
        );
        let known = known_app_keys_from_ndr(owner, &applied.app_keys, applied.created_at);
        if current.as_ref() == Some(&known) {
            return None;
        }
        self.app_keys.insert(owner_hex, known);
        Some((applied.app_keys, applied.created_at))
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
            return LocalAuthorizationState::Authorized;
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

fn load_or_create_local_invite(
    storage: &dyn StorageAdapter,
    device_pubkey: PublicKey,
    device_id: &str,
    owner_pubkey: PublicKey,
) -> anyhow::Result<Invite> {
    let storage_key = format!("device-invite/{device_id}");
    if let Some(serialized) = storage.get(&storage_key)? {
        if let Ok(mut invite) = Invite::deserialize(&serialized) {
            invite.owner_public_key = Some(owner_pubkey);
            return Ok(invite);
        }
    }

    let mut invite = Invite::create_new(device_pubkey, Some(device_id.to_string()), None)?;
    invite.owner_public_key = Some(owner_pubkey);
    storage.put(&storage_key, invite.serialize()?)?;
    Ok(invite)
}

fn parse_link_device_invite_input(input: &str, owner_pubkey: PublicKey) -> anyhow::Result<Invite> {
    let invite = parse_public_invite_input(input)?;
    if invite.purpose.as_deref() != Some("link") {
        return Err(anyhow::anyhow!("Invalid link code."));
    }
    if invite
        .owner_public_key
        .is_some_and(|invite_owner| invite_owner != owner_pubkey)
    {
        return Err(anyhow::anyhow!("This code is for a different profile."));
    }
    Ok(invite)
}

pub(super) fn known_app_keys_to_ndr(known: &KnownAppKeys) -> Option<AppKeys> {
    Some(AppKeys::new(
        known
            .devices
            .iter()
            .filter_map(|device| {
                PublicKey::parse(&device.identity_pubkey_hex)
                    .ok()
                    .map(|pubkey| DeviceEntry::new(pubkey, device.created_at_secs))
            })
            .collect(),
    ))
}

pub(super) fn known_app_keys_from_ndr(
    owner: PublicKey,
    app_keys: &AppKeys,
    created_at_secs: u64,
) -> KnownAppKeys {
    let mut devices = app_keys
        .get_all_devices()
        .into_iter()
        .map(|device| KnownAppKeyDevice {
            identity_pubkey_hex: device.identity_pubkey.to_hex(),
            created_at_secs: device.created_at,
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| left.identity_pubkey_hex.cmp(&right.identity_pubkey_hex));
    KnownAppKeys {
        owner_pubkey_hex: owner.to_hex(),
        created_at_secs,
        devices,
    }
}
