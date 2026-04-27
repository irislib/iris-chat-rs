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

        self.push_debug_log("app.foreground", "refresh relay session");
        self.schedule_session_connect();
        self.request_protocol_subscription_refresh_forced();
        self.fetch_recent_protocol_state();
        self.fetch_recent_messages_for_tracked_peers(unix_now());
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

        let owner_pubkey = logged_in.owner_pubkey;
        self.upsert_local_app_key_device(owner_pubkey, device_pubkey);
        self.publish_local_app_keys();
        self.rebuild_state();
        self.persist_best_effort();
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
        _allow_protocol_restore: bool,
    ) -> anyhow::Result<()> {
        self.push_debug_log(
            "session.start",
            format!(
                "owner={} has_owner_keys={} allow_restore={}",
                owner_pubkey.to_hex(),
                owner_keys.is_some(),
                allow_restore,
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
        self.active_chat_id = None;
        self.screen_stack.clear();
        self.owner_profiles.clear();
        self.app_keys.clear();
        self.groups.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.direct_message_subscriptions = DirectMessageSubscriptionTracker::new();
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
            self.preferences.startup_at_login_enabled =
                persisted.preferences.startup_at_login_enabled;
            self.preferences.nostr_relay_urls =
                migrate_default_nostr_relay_urls(&persisted.preferences.nostr_relay_urls);
            self.preferences.image_proxy_enabled = persisted.preferences.image_proxy_enabled;
            self.preferences.image_proxy_url = persisted.preferences.image_proxy_url.clone();
            self.preferences.image_proxy_key_hex =
                persisted.preferences.image_proxy_key_hex.clone();
            self.preferences.image_proxy_salt_hex =
                persisted.preferences.image_proxy_salt_hex.clone();
            self.preferences.mobile_push_server_url =
                persisted.preferences.mobile_push_server_url.clone();
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
                .map(|group| (group.id.clone(), group))
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
                                    }
                                })
                                .collect(),
                        },
                    )
                })
                .collect();
        }

        let previous_authorization_state = persisted
            .as_ref()
            .and_then(|state| state.authorization_state.clone())
            .map(LocalAuthorizationState::from);

        let device_pubkey = device_keys.public_key();
        if owner_keys.is_some() {
            self.upsert_local_app_key_device(owner_pubkey, device_pubkey);
        }

        let storage = Arc::new(FileStorageAdapter::new(
            self.ndr_storage_dir(owner_pubkey, device_pubkey),
        )?) as Arc<dyn StorageAdapter>;
        let device_id = device_pubkey.to_hex();
        let local_invite =
            load_or_create_local_invite(storage.as_ref(), device_pubkey, &device_id, owner_pubkey)?;
        let ndr_runtime = NdrRuntime::new(
            device_pubkey,
            device_keys.secret_key().to_secret_bytes(),
            device_id,
            owner_pubkey,
            Some(storage),
            Some(local_invite.clone()),
        );
        ndr_runtime.init()?;
        ndr_runtime.set_auto_adopt_chat_settings(true);

        for app_keys in self.app_keys.values() {
            if let (Ok(owner), Some(keys)) = (
                PublicKey::parse(&app_keys.owner_pubkey_hex),
                known_app_keys_to_ndr(app_keys),
            ) {
                ndr_runtime.ingest_app_keys_snapshot(owner, keys, app_keys.created_at_secs);
            }
        }
        ndr_runtime.sync_groups(self.groups.values().cloned().collect())?;

        let authorization_state = self.local_authorization_state(
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
        self.runtime
            .block_on(ensure_session_relays_configured(&client, &relay_urls));
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

        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.start_relay_status_watchers();
        self.schedule_session_connect();
        self.emit_account_bundle_update(owner_keys.as_ref(), &device_keys);
        self.republish_local_identity_artifacts();
        self.process_runtime_events();
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
        if !entry
            .devices
            .iter()
            .any(|existing| existing.identity_pubkey_hex == device.to_hex())
        {
            entry.devices.push(KnownAppKeyDevice {
                identity_pubkey_hex: device.to_hex(),
                created_at_secs: now,
            });
        }
        entry.created_at_secs = now;
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

    pub(super) fn local_authorization_state(
        &self,
        owner_keys: Option<&Keys>,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
        previous: Option<LocalAuthorizationState>,
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
