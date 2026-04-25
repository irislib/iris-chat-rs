use super::*;

impl AppCore {
    pub(super) fn create_account(&mut self, name: &str) {
        self.state.busy.creating_account = true;
        self.emit_state();

        let owner_keys = Keys::generate();
        let device_keys = Keys::generate();
        let trimmed_name = name.trim().to_string();

        if let Err(error) = self.start_primary_session(owner_keys, device_keys, false, false) {
            self.state.toast = Some(error.to_string());
        } else if !trimmed_name.is_empty() {
            self.set_local_profile_name(&trimmed_name);
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
        self.push_debug_log("app.foreground", "refresh relay session");
        self.schedule_session_connect();
        self.request_protocol_subscription_refresh_forced();
        self.fetch_recent_protocol_state();
        self.fetch_recent_messages_for_tracked_peers(now);
        if self.can_poll_pending_device_invites() {
            self.fetch_pending_device_invites_for_local_owner();
            self.schedule_pending_device_invite_poll(Duration::from_secs(
                DEVICE_INVITE_DISCOVERY_POLL_SECS,
            ));
        }

        for pending in &mut self.pending_outbound {
            if !pending.in_flight {
                pending.next_retry_at_secs = now.get();
            }
        }
        for pending in &mut self.pending_group_controls {
            if !pending.in_flight {
                pending.next_retry_at_secs = now.get();
            }
        }
        self.retry_pending_outbound(now);
        self.retry_pending_group_controls(now);
        self.schedule_next_pending_retry(now.get());
        self.state.busy.syncing_network = true;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn restore_primary_session(&mut self, owner_nsec: &str) {
        self.state.busy.restoring_session = true;
        self.emit_state();

        let result = Keys::parse(owner_nsec.trim())
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .and_then(|owner_keys| {
                self.start_primary_session(owner_keys, Keys::generate(), true, false)
            });

        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
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
                    let keys = Keys::parse(secret.trim())
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    let derived_owner = OwnerPubkey::from_bytes(keys.public_key().to_bytes());
                    if derived_owner != owner_pubkey {
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
            self.start_session(owner_pubkey, owner_keys, device_keys, true, true)
        })();

        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        }

        self.state.busy.restoring_session = false;
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn start_linked_device(&mut self, owner_input: &str) {
        self.push_debug_log(
            "session.start_linked",
            format!("owner_input={}", owner_input.trim()),
        );
        self.state.busy.linking_device = true;
        self.emit_state();

        let result = parse_owner_input(owner_input).and_then(|owner_pubkey| {
            self.start_session(owner_pubkey, None, Keys::generate(), false, false)
        });
        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        }

        self.state.busy.linking_device = false;
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn logout(&mut self) {
        self.push_debug_log("session.logout", "clearing runtime state");
        let previous_rev = self.state.rev;
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
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
        self.pending_inbound.clear();
        self.pending_outbound.clear();
        self.pending_group_controls.clear();
        self.owner_profiles.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.next_message_id = 1;
        self.state = AppState::empty();
        self.state.rev = previous_rev;
        self.clear_persistence_best_effort();
        self.emit_state();
    }

    pub(super) fn add_authorized_device(&mut self, device_input: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        };
        if logged_in.owner_keys.is_none() {
            self.state.toast = Some("Only the primary device can manage devices.".to_string());
            self.emit_state();
            return;
        }

        let Ok(device_pubkey) = parse_device_input(device_input) else {
            self.state.toast = Some("Invalid device key.".to_string());
            self.emit_state();
            return;
        };
        if device_pubkey == local_device_from_keys(&logged_in.device_keys) {
            self.state.toast = Some("The current device is already authorized.".to_string());
            self.emit_state();
            return;
        }

        self.state.busy.updating_roster = true;
        self.emit_state();

        let now = unix_now();
        let updated_roster = {
            let logged_in = self.logged_in.as_mut().expect("checked above");
            let current_roster = local_roster_from_session_manager(&logged_in.session_manager);
            let mut editor = RosterEditor::from_roster(current_roster.as_ref());
            editor.authorize_device(device_pubkey, now);
            let roster = editor.build(now);
            logged_in.session_manager.apply_local_roster(roster.clone());
            logged_in.authorization_state = derive_local_authorization_state(
                logged_in.owner_keys.is_some(),
                logged_in.owner_pubkey,
                local_device_from_keys(&logged_in.device_keys),
                &logged_in.session_manager,
                Some(logged_in.authorization_state),
            );
            roster
        };

        self.publish_roster_update(updated_roster);
        self.request_protocol_subscription_refresh();
        self.persist_best_effort();
        self.state.busy.updating_roster = false;
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn remove_authorized_device(&mut self, device_pubkey_hex: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore an account first.".to_string());
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
        if device_pubkey == local_device_from_keys(&logged_in.device_keys) {
            self.state.toast = Some("The current device cannot remove itself.".to_string());
            self.emit_state();
            return;
        }

        self.state.busy.updating_roster = true;
        self.emit_state();

        let now = unix_now();
        let updated_roster = {
            let logged_in = self.logged_in.as_mut().expect("checked above");
            let current_roster = local_roster_from_session_manager(&logged_in.session_manager);
            let mut editor = RosterEditor::from_roster(current_roster.as_ref());
            editor.revoke_device(device_pubkey);
            let roster = editor.build(now);
            logged_in.session_manager.apply_local_roster(roster.clone());
            logged_in.authorization_state = derive_local_authorization_state(
                logged_in.owner_keys.is_some(),
                logged_in.owner_pubkey,
                local_device_from_keys(&logged_in.device_keys),
                &logged_in.session_manager,
                Some(logged_in.authorization_state),
            );
            roster
        };

        self.publish_roster_update(updated_roster);
        self.request_protocol_subscription_refresh();
        self.persist_best_effort();
        self.state.busy.updating_roster = false;
        self.rebuild_state();
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
        self.push_debug_log(
            "session.start_primary",
            format!(
                "owner_pubkey={} allow_restore={} allow_protocol_restore={}",
                owner_keys.public_key().to_hex(),
                allow_restore,
                allow_protocol_restore,
            ),
        );
        let owner_pubkey = OwnerPubkey::from_bytes(owner_keys.public_key().to_bytes());
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
                owner_pubkey,
                owner_keys.is_some(),
                allow_restore,
                allow_protocol_restore,
            ),
        );
        if let Some(existing) = self.logged_in.take() {
            let client = existing.client;
            self.runtime.spawn(async move {
                client.unsubscribe_all().await;
                let _ = client.shutdown().await;
            });
        }

        self.threads.clear();
        self.pending_inbound.clear();
        self.active_chat_id = None;
        self.screen_stack.clear();
        self.pending_outbound.clear();
        self.pending_group_controls.clear();
        self.owner_profiles.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.debug_log.clear();
        self.debug_event_counters = DebugEventCounters::default();
        self.next_message_id = 1;
        self.chat_message_ttl_seconds.clear();

        let device_secret_bytes = device_keys.secret_key().to_secret_bytes();
        let local_device = DevicePubkey::from_bytes(device_keys.public_key().to_bytes());
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
        let persisted_authorization_state = persisted
            .as_ref()
            .and_then(|persisted| persisted.authorization_state.clone())
            .map(Into::into);

        if let Some(persisted) = &persisted {
            self.active_chat_id = persisted.active_chat_id.clone();
            self.next_message_id = persisted.next_message_id.max(1);
            self.owner_profiles = persisted.owner_profiles.clone();
            self.chat_message_ttl_seconds = persisted.chat_message_ttl_seconds.clone();
            self.preferences.send_typing_indicators = persisted.preferences.send_typing_indicators;
            self.preferences.send_read_receipts = persisted.preferences.send_read_receipts;
            self.preferences.desktop_notifications_enabled =
                persisted.preferences.desktop_notifications_enabled;
            self.preferences.startup_at_login_enabled =
                persisted.preferences.startup_at_login_enabled;
            self.preferences.nostr_relay_urls =
                normalize_nostr_relay_urls(&persisted.preferences.nostr_relay_urls);
            self.preferences.image_proxy_enabled = persisted.preferences.image_proxy_enabled;
            self.preferences.image_proxy_url = persisted.preferences.image_proxy_url.clone();
            self.preferences.image_proxy_key_hex =
                persisted.preferences.image_proxy_key_hex.clone();
            self.preferences.image_proxy_salt_hex =
                persisted.preferences.image_proxy_salt_hex.clone();
            if allow_protocol_restore {
                self.pending_outbound = persisted.pending_outbound.clone();
                for pending in &mut self.pending_outbound {
                    pending.publish_mode = migrate_publish_mode(
                        pending.publish_mode.clone(),
                        pending.prepared_publish.as_ref(),
                    );
                    if pending.in_flight {
                        pending.in_flight = false;
                        pending.next_retry_at_secs = now.get();
                    }
                }
                self.pending_group_controls = persisted.pending_group_controls.clone();
                for pending in &mut self.pending_group_controls {
                    if pending.in_flight {
                        pending.in_flight = false;
                        pending.next_retry_at_secs = now.get();
                    }
                }
                self.pending_inbound = persisted.pending_inbound.clone();
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
            }
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
                                        author: message.author.clone(),
                                        body,
                                        attachments: if message.attachments.is_empty() {
                                            parsed_attachments
                                        } else {
                                            message.attachments.clone()
                                        },
                                        reactions: message.reactions.clone(),
                                        is_outgoing: message.is_outgoing,
                                        created_at_secs: message.created_at_secs,
                                        expires_at_secs: message.expires_at_secs,
                                        delivery: message.delivery.clone().into(),
                                    }
                                })
                                .collect(),
                        },
                    )
                })
                .collect();
        }

        let persisted_session_manager = persisted.as_ref().and_then(|persisted| {
            if allow_protocol_restore {
                persisted.session_manager.clone()
            } else {
                None
            }
        });

        let mut session_manager = persisted_session_manager
            .filter(|snapshot| {
                snapshot.local_owner_pubkey == owner_pubkey
                    && snapshot.local_device_pubkey == local_device
            })
            .map(|snapshot| SessionManager::from_snapshot(snapshot, device_secret_bytes))
            .transpose()?
            .unwrap_or_else(|| SessionManager::new(owner_pubkey, device_secret_bytes));

        let group_manager = persisted
            .as_ref()
            .and_then(|persisted| persisted.group_manager.clone())
            .filter(|snapshot| snapshot.local_owner_pubkey == owner_pubkey)
            .map(GroupManager::from_snapshot)
            .transpose()?
            .unwrap_or_else(|| GroupManager::new(owner_pubkey));

        let existing_local_roster = session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner_pubkey)
            .and_then(|user| user.roster);
        if owner_keys.is_some() && existing_local_roster.is_none() {
            let mut roster_editor = RosterEditor::new();
            roster_editor.authorize_device(local_device, now);
            session_manager.apply_local_roster(roster_editor.build(now));
        }

        let authorization_state = derive_local_authorization_state(
            owner_keys.is_some(),
            owner_pubkey,
            local_device,
            &session_manager,
            persisted_authorization_state,
        );
        self.push_debug_log(
            "session.authorization",
            format!("state={authorization_state:?} owner={owner_pubkey} device={local_device}"),
        );

        if authorization_state != LocalAuthorizationState::Revoked {
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            session_manager.ensure_local_invite(&mut ctx)?;
        }

        if authorization_state != LocalAuthorizationState::Authorized {
            self.active_chat_id = None;
            self.screen_stack.clear();
            self.pending_inbound.clear();
            self.pending_outbound.clear();
            self.pending_group_controls.clear();
            self.chat_message_ttl_seconds.clear();
        } else if let Some(chat_id) = self.active_chat_id.clone() {
            self.screen_stack = vec![Screen::Chat { chat_id }];
        }

        let client = Client::new(device_keys.clone());
        let relay_urls = relay_urls_from_strings(&self.preferences.nostr_relay_urls);
        self.runtime
            .block_on(ensure_session_relays_configured(&client, &relay_urls));
        self.start_notifications_loop(client.clone());

        self.logged_in = Some(LoggedInState {
            owner_pubkey,
            owner_keys: owner_keys.clone(),
            device_keys: device_keys.clone(),
            client,
            relay_urls,
            session_manager,
            group_manager,
            authorization_state,
        });
        self.schedule_pending_device_invite_poll(Duration::from_secs(
            DEVICE_INVITE_DISCOVERY_POLL_SECS,
        ));
        self.schedule_session_connect();

        self.emit_account_bundle_update(owner_keys.as_ref(), &device_keys);
        self.republish_local_identity_artifacts();
        self.reconcile_recent_handshake_peers();
        self.retry_pending_inbound(now);
        self.retry_pending_outbound(now);
        self.retry_pending_group_controls(now);
        self.schedule_next_pending_retry(now.get());
        self.state.busy.syncing_network = true;
        self.rebuild_state();
        self.persist_best_effort();
        self.request_protocol_subscription_refresh();
        if authorization_state != LocalAuthorizationState::Revoked {
            self.schedule_tracked_peer_catch_up(Duration::from_secs(
                RESUBSCRIBE_CATCH_UP_DELAY_SECS,
            ));
        }
        self.emit_state();
        Ok(())
    }
}
