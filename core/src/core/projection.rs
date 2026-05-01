use super::*;

/// Compare two `AppState` snapshots ignoring `rev`. Returns true if the UI
/// would render identically.
fn state_content_eq(a: &AppState, b: &AppState) -> bool {
    a.router == b.router
        && a.account == b.account
        && a.device_roster == b.device_roster
        && a.busy == b.busy
        && a.chat_list == b.chat_list
        && a.current_chat == b.current_chat
        && a.group_details == b.group_details
        && a.public_invite == b.public_invite
        && a.link_device == b.link_device
        && a.network_status == b.network_status
        && a.mobile_push == b.mobile_push
        && a.preferences == b.preferences
        && a.toast == b.toast
}

impl AppCore {
    pub(super) fn rebuild_state(&mut self) {
        if self.batch_depth > 0 {
            self.batch_dirty_state = true;
            return;
        }
        self.rebuild_state_inner();
    }

    fn rebuild_state_inner(&mut self) {
        let t0 = crate::perflog::now_ms();
        self.state.account = self.build_account_snapshot();
        self.state.device_roster = self.build_device_roster_snapshot();
        self.refresh_relay_connection_status();
        self.state.network_status = Some(self.build_network_status_snapshot());
        self.state.public_invite = self.build_public_invite_snapshot();
        self.state.link_device = self.build_link_device_snapshot();
        // Mobile push snapshot drives the FCM/APNs push subscription
        // author list. Recompute every rebuild so newly tracked DM
        // peers / group senders (which the tracker exposes lazily via
        // `known_message_author_hexes`) are reflected immediately on
        // mobile. The historical heavy `sessions` vec is gone, so
        // building this is just a HashSet walk + sort.
        let next_mobile_push = self.build_mobile_push_sync_snapshot();
        if next_mobile_push != self.cached_mobile_push {
            self.cached_mobile_push = next_mobile_push;
        }
        self.mobile_push_dirty = false;
        self.state.mobile_push = self.cached_mobile_push.clone();
        self.state.preferences = self.preferences.clone();

        let default_screen = match self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.authorization_state)
        {
            None => Screen::Welcome,
            Some(LocalAuthorizationState::Authorized) => Screen::ChatList,
            Some(LocalAuthorizationState::AwaitingApproval) => Screen::AwaitingDeviceApproval,
            Some(LocalAuthorizationState::Revoked) => Screen::DeviceRevoked,
        };

        self.prune_expired_typing_indicators();
        let mut threads: Vec<&ThreadRecord> = self.threads.values().collect();
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at_secs));

        self.state.chat_list = threads
            .iter()
            .map(|thread| {
                let last_message = thread.messages.last();
                let thread_kind = chat_kind_for_id(&thread.chat_id);
                let group_snapshot = self.group_snapshot_for_chat_id(&thread.chat_id);
                let is_muted = self.is_chat_muted(&thread.chat_id);
                let display_name = group_snapshot
                    .as_ref()
                    .map(|group| group.name.clone())
                    .unwrap_or_else(|| self.owner_display_label(&thread.chat_id));
                let subtitle = if group_snapshot.is_some() {
                    None
                } else {
                    self.owner_secondary_identifier(&thread.chat_id)
                };
                let member_count = group_snapshot
                    .as_ref()
                    .map(|group| group.members.len() as u64)
                    .unwrap_or(0);
                let direct_picture = if group_snapshot.is_none() {
                    self.owner_profiles
                        .get(&thread.chat_id)
                        .and_then(|profile| profile.picture.clone())
                } else {
                    None
                };
                ChatThreadSnapshot {
                    chat_id: thread.chat_id.clone(),
                    kind: thread_kind,
                    display_name,
                    subtitle,
                    picture_url: group_snapshot
                        .as_ref()
                        .and_then(|group| group.picture.clone())
                        .or(direct_picture),
                    member_count,
                    last_message_preview: last_message.map(message_preview),
                    last_message_at_secs: last_message.map(|message| message.created_at_secs),
                    last_message_is_outgoing: last_message.map(|message| message.is_outgoing),
                    last_message_delivery: last_message.map(|message| message.delivery.clone()),
                    unread_count: thread.unread_count,
                    is_typing: self.thread_has_typing_indicator(&thread.chat_id),
                    is_muted,
                }
            })
            .collect();

        self.state.current_chat = self
            .active_chat_id
            .as_ref()
            .and_then(|chat_id| self.threads.get(chat_id))
            .map(|thread| {
                let group_snapshot = self.group_snapshot_for_chat_id(&thread.chat_id);
                let is_muted = self.is_chat_muted(&thread.chat_id);
                let direct_picture = if group_snapshot.is_none() {
                    self.owner_profiles
                        .get(&thread.chat_id)
                        .and_then(|profile| profile.picture.clone())
                } else {
                    None
                };
                CurrentChatSnapshot {
                    chat_id: thread.chat_id.clone(),
                    kind: chat_kind_for_id(&thread.chat_id),
                    display_name: group_snapshot
                        .as_ref()
                        .map(|group| group.name.clone())
                        .unwrap_or_else(|| self.owner_display_label(&thread.chat_id)),
                    subtitle: group_snapshot
                        .as_ref()
                        .map(|group| format!("{} members", group.members.len()))
                        .or_else(|| self.owner_secondary_identifier(&thread.chat_id)),
                    picture_url: group_snapshot
                        .as_ref()
                        .and_then(|group| group.picture.clone())
                        .or(direct_picture),
                    group_id: group_snapshot.as_ref().map(|group| group.id.clone()),
                    member_count: group_snapshot
                        .as_ref()
                        .map(|group| group.members.len() as u64)
                        .unwrap_or(0),
                    message_ttl_seconds: self
                        .chat_message_ttl_seconds
                        .get(&thread.chat_id)
                        .copied(),
                    is_muted,
                    messages: thread.messages.clone(),
                    typing_indicators: self.typing_indicator_snapshots(&thread.chat_id),
                }
            });

        self.state.group_details = self.screen_stack.last().and_then(|screen| match screen {
            Screen::GroupDetails { group_id } => self.build_group_details_snapshot(group_id),
            _ => None,
        });

        self.state.router = Router {
            default_screen,
            screen_stack: self.screen_stack.clone(),
        };
        crate::perflog!(
            "rebuild_state ms={} threads={} cur_msgs={}",
            crate::perflog::now_ms().saturating_sub(t0),
            self.threads.len(),
            self.state
                .current_chat
                .as_ref()
                .map(|c| c.messages.len())
                .unwrap_or(0)
        );
    }

    pub(super) fn prune_expired_typing_indicators(&mut self) {
        let now = unix_now().get();
        let latest_message_secs_by_chat = self.latest_message_secs_by_chat();
        self.typing_indicators.retain(|_, indicator| {
            typing_indicator_is_active(indicator, now, &latest_message_secs_by_chat)
        });
    }

    pub(super) fn thread_has_typing_indicator(&self, chat_id: &str) -> bool {
        let now = unix_now().get();
        let latest_message_secs = self.latest_message_secs_for_chat(chat_id);
        self.typing_indicators.values().any(|indicator| {
            indicator.chat_id == chat_id
                && indicator.expires_at_secs > now
                && indicator.last_event_secs > latest_message_secs
        })
    }

    pub(super) fn typing_indicator_snapshots(&self, chat_id: &str) -> Vec<TypingIndicatorSnapshot> {
        let now = unix_now().get();
        let latest_message_secs = self.latest_message_secs_for_chat(chat_id);
        let mut indicators = self
            .typing_indicators
            .values()
            .filter(|indicator| {
                indicator.chat_id == chat_id
                    && indicator.expires_at_secs > now
                    && indicator.last_event_secs > latest_message_secs
            })
            .map(|indicator| TypingIndicatorSnapshot {
                chat_id: indicator.chat_id.clone(),
                display_name: self.owner_display_label(&indicator.author_owner_hex),
                expires_at_secs: indicator.expires_at_secs,
            })
            .collect::<Vec<_>>();
        indicators.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        indicators
    }

    fn latest_message_secs_for_chat(&self, chat_id: &str) -> u64 {
        self.threads
            .get(chat_id)
            .and_then(|thread| thread.messages.last())
            .map(|message| message.created_at_secs)
            .unwrap_or(0)
    }

    fn latest_message_secs_by_chat(&self) -> BTreeMap<String, u64> {
        self.threads
            .iter()
            .filter_map(|(chat_id, thread)| {
                thread
                    .messages
                    .last()
                    .map(|message| (chat_id.clone(), message.created_at_secs))
            })
            .collect()
    }

    pub(super) fn build_account_snapshot(&self) -> Option<AccountSnapshot> {
        let logged_in = self.logged_in.as_ref()?;
        let owner_public_key_hex = logged_in.owner_pubkey.to_hex();
        let owner_npub = owner_npub_from_owner(logged_in.owner_pubkey)
            .unwrap_or_else(|| owner_public_key_hex.clone());
        let display_name = self.owner_display_label(&owner_public_key_hex);
        let picture_url = self
            .owner_profiles
            .get(&owner_public_key_hex)
            .and_then(|profile| profile.picture.clone());
        let device_public_key_hex = logged_in.device_keys.public_key().to_hex();
        let device_npub = logged_in
            .device_keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| device_public_key_hex.clone());

        Some(AccountSnapshot {
            public_key_hex: owner_public_key_hex,
            npub: owner_npub,
            display_name,
            picture_url,
            device_public_key_hex,
            device_npub,
            has_owner_signing_authority: logged_in.owner_keys.is_some(),
            authorization_state: public_authorization_state(logged_in.authorization_state),
        })
    }

    pub(super) fn build_device_roster_snapshot(&self) -> Option<DeviceRosterSnapshot> {
        let logged_in = self.logged_in.as_ref()?;
        let account = self.build_account_snapshot()?;
        let current_device_pubkey_hex = account.device_public_key_hex.clone();
        let current_device_npub = account.device_npub.clone();
        let mut entries = BTreeMap::<String, DeviceEntrySnapshot>::new();

        if let Some(app_keys) = self.app_keys.get(&logged_in.owner_pubkey.to_hex()) {
            for device in &app_keys.devices {
                let device_pubkey_hex = device.identity_pubkey_hex.clone();
                entries.insert(
                    device_pubkey_hex.clone(),
                    DeviceEntrySnapshot {
                        device_pubkey_hex: device_pubkey_hex.clone(),
                        device_npub: device_npub(&device_pubkey_hex)
                            .unwrap_or_else(|| device_pubkey_hex.clone()),
                        is_current_device: device_pubkey_hex == current_device_pubkey_hex,
                        is_authorized: true,
                        is_stale: false,
                        last_activity_secs: Some(device.created_at_secs),
                    },
                );
            }
        }

        entries
            .entry(current_device_pubkey_hex.clone())
            .or_insert(DeviceEntrySnapshot {
                device_pubkey_hex: current_device_pubkey_hex.clone(),
                device_npub: current_device_npub.clone(),
                is_current_device: true,
                is_authorized: matches!(
                    logged_in.authorization_state,
                    LocalAuthorizationState::Authorized
                ),
                is_stale: matches!(
                    logged_in.authorization_state,
                    LocalAuthorizationState::Revoked
                ),
                last_activity_secs: None,
            });

        let mut devices = entries.into_values().collect::<Vec<_>>();
        devices.sort_by(|left, right| {
            right
                .is_current_device
                .cmp(&left.is_current_device)
                .then_with(|| left.device_pubkey_hex.cmp(&right.device_pubkey_hex))
        });

        Some(DeviceRosterSnapshot {
            owner_public_key_hex: account.public_key_hex,
            owner_npub: account.npub,
            current_device_public_key_hex: current_device_pubkey_hex,
            current_device_npub,
            can_manage_devices: logged_in.owner_keys.is_some(),
            authorization_state: public_authorization_state(logged_in.authorization_state),
            devices,
        })
    }

    pub(super) fn build_network_status_snapshot(&self) -> NetworkStatusSnapshot {
        let recent_event_count = self.debug_event_counters.app_keys_events
            + self.debug_event_counters.invite_events
            + self.debug_event_counters.invite_response_events
            + self.debug_event_counters.message_events
            + self.debug_event_counters.group_events
            + self.debug_event_counters.other_events;
        let last_debug = self.debug_log.back();

        NetworkStatusSnapshot {
            relay_set_id: RELAY_SET_ID.to_string(),
            relay_urls: self.preferences.nostr_relay_urls.clone(),
            relay_connections: self.build_relay_connection_snapshots(),
            connected_relay_count: self.relay_connected_count,
            all_relays_offline_since_secs: self.all_relays_offline_since_secs,
            syncing: self.state.busy.syncing_network,
            pending_outbound_count: self.pending_relay_publishes.len() as u64,
            pending_group_control_count: 0,
            recent_event_count,
            recent_log_count: self.debug_log.len() as u64,
            last_debug_category: last_debug.map(|entry| entry.category.clone()),
            last_debug_detail: last_debug.map(|entry| entry.detail.clone()),
        }
    }

    fn build_relay_connection_snapshots(&self) -> Vec<RelayConnectionSnapshot> {
        let relay_statuses = self
            .logged_in
            .as_ref()
            .map(|logged_in| {
                self.runtime.block_on(async {
                    logged_in
                        .client
                        .relays()
                        .await
                        .into_iter()
                        .map(|(url, relay)| {
                            let url = normalize_nostr_relay_url(&url.to_string())
                                .unwrap_or_else(|_| url.to_string());
                            (url, relay_connection_status(relay.status()).to_string())
                        })
                        .collect::<BTreeMap<_, _>>()
                })
            })
            .unwrap_or_default();

        self.preferences
            .nostr_relay_urls
            .iter()
            .map(|url| RelayConnectionSnapshot {
                url: url.clone(),
                status: relay_statuses
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| "offline".to_string()),
            })
            .collect()
    }

    pub(super) fn build_public_invite_snapshot(&self) -> Option<PublicInviteSnapshot> {
        let invite = &self.logged_in.as_ref()?.local_invite;
        let url = invite.get_url(CHAT_INVITE_ROOT_URL).ok()?;
        Some(PublicInviteSnapshot { url })
    }

    pub(super) fn build_link_device_snapshot(&self) -> Option<LinkDeviceSnapshot> {
        let pending = self.pending_linked_device.as_ref()?;
        Some(LinkDeviceSnapshot {
            url: pending.url.clone(),
            device_input: pending.device_keys.public_key().to_bech32().ok()?,
        })
    }

    pub(super) fn group_snapshot_for_chat_id(&self, chat_id: &str) -> Option<GroupData> {
        let group_id = parse_group_id_from_chat_id(chat_id)?;
        self.groups.get(&group_id).cloned()
    }

    pub(super) fn build_group_details_snapshot(
        &self,
        group_id: &str,
    ) -> Option<GroupDetailsSnapshot> {
        let logged_in = self.logged_in.as_ref()?;
        let group = self.groups.get(group_id)?.clone();
        let local_owner_hex = logged_in.owner_pubkey.to_hex();
        let mut members = group
            .members
            .iter()
            .map(|owner_hex| {
                let owner = PublicKey::parse(owner_hex).ok();
                GroupMemberSnapshot {
                    owner_pubkey_hex: owner_hex.clone(),
                    display_name: self.owner_display_label(owner_hex),
                    npub: owner
                        .and_then(owner_npub_from_owner)
                        .unwrap_or_else(|| owner_hex.clone()),
                    is_admin: group.admins.iter().any(|admin| admin == owner_hex),
                    is_creator: group.admins.first() == Some(owner_hex),
                    is_local_owner: owner_hex == &local_owner_hex,
                }
            })
            .collect::<Vec<_>>();
        members.sort_by(|left, right| {
            right
                .is_local_owner
                .cmp(&left.is_local_owner)
                .then_with(|| right.is_creator.cmp(&left.is_creator))
                .then_with(|| right.is_admin.cmp(&left.is_admin))
                .then_with(|| left.owner_pubkey_hex.cmp(&right.owner_pubkey_hex))
        });

        let creator = group
            .admins
            .first()
            .cloned()
            .unwrap_or_else(|| local_owner_hex.clone());
        let creator_npub = PublicKey::parse(&creator)
            .ok()
            .and_then(owner_npub_from_owner)
            .unwrap_or_else(|| creator.clone());
        let is_muted = self.is_chat_muted(&group_chat_id(&group.id));

        Some(GroupDetailsSnapshot {
            group_id: group.id,
            name: group.name,
            picture_url: group.picture,
            created_by_display_name: self.owner_display_label(&creator),
            created_by_npub: creator_npub,
            can_manage: group.admins.iter().any(|admin| admin == &local_owner_hex),
            is_muted,
            revision: group.created_at,
            members,
        })
    }

    pub(super) fn can_use_chats(&self) -> bool {
        matches!(
            self.logged_in
                .as_ref()
                .map(|logged_in| logged_in.authorization_state),
            Some(LocalAuthorizationState::Authorized)
        )
    }

    pub(super) fn emit_account_bundle_update(&self, owner_keys: Option<&Keys>, device_keys: &Keys) {
        let device_nsec = device_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| device_keys.secret_key().to_secret_hex());
        let owner_nsec = owner_keys.map(|keys| {
            keys.secret_key()
                .to_bech32()
                .unwrap_or_else(|_| keys.secret_key().to_secret_hex())
        });
        let owner_pubkey_hex = owner_keys
            .map(|keys| keys.public_key().to_hex())
            .or_else(|| {
                self.logged_in
                    .as_ref()
                    .map(|logged_in| logged_in.owner_pubkey.to_hex())
            })
            .unwrap_or_default();
        let _ = self.update_tx.send(AppUpdate::PersistAccountBundle {
            rev: self.state.rev,
            owner_nsec,
            owner_pubkey_hex,
            device_nsec,
        });
    }

    pub(super) fn emit_state(&mut self) {
        if self.batch_depth > 0 {
            self.batch_dirty_state = true;
            return;
        }
        self.emit_state_inner();
    }

    fn emit_state_inner(&mut self) {
        // Skip the push if nothing user-visible changed since the last
        // emit. Compare every field except `rev`, which we own and bump
        // ourselves. On Android debug each FullState push triggers a
        // full Compose recomposition of ChatScreen (~400-1000 ms of UI
        // thread). The post-OpenChat cascade (SyncComplete →
        // FetchCatchUpEvents → ...) was producing 3-4 redundant pushes
        // and >1 s of Skipped frames each time.
        if let Some(last) = self.last_emitted_state.as_ref() {
            if state_content_eq(last, &self.state) {
                return;
            }
        }

        self.state.rev = self.state.rev.saturating_add(1);
        let t0 = crate::perflog::now_ms();
        let snapshot = self.state.clone();
        let t_clone1 = crate::perflog::now_ms();
        match self.shared_state.write() {
            Ok(mut slot) => *slot = snapshot.clone(),
            Err(poison) => *poison.into_inner() = snapshot.clone(),
        }
        let t_shared = crate::perflog::now_ms();
        self.last_emitted_state = Some(snapshot.clone());
        let _ = self.update_tx.send(AppUpdate::FullState(snapshot));
        crate::perflog!(
            "emit_state rev={} clone_ms={} shared_write_ms={} total_ms={} chats={} cur_chat_msgs={}",
            self.state.rev,
            t_clone1.saturating_sub(t0),
            t_shared.saturating_sub(t_clone1),
            crate::perflog::now_ms().saturating_sub(t0),
            self.state.chat_list.len(),
            self.state.current_chat.as_ref().map(|c| c.messages.len()).unwrap_or(0)
        );
    }

    /// Mark mobile push state as affected by the current mutation.
    /// Rebuilds recompute the lightweight author snapshot immediately,
    /// but callers still use this as the semantic marker for changes
    /// that can alter the push subscription body.
    pub(super) fn mark_mobile_push_dirty(&mut self) {
        self.mobile_push_dirty = true;
    }

    /// Enter a batch scope. While `batch_depth > 0`, calls to
    /// `rebuild_state` / `emit_state` / `persist_best_effort` are deferred
    /// and coalesced into a single rebuild + persist + emit at the
    /// outermost `exit_batch()`.
    pub(super) fn enter_batch(&mut self) {
        self.batch_depth = self.batch_depth.saturating_add(1);
    }

    pub(super) fn exit_batch(&mut self) {
        if self.batch_depth == 0 {
            return;
        }
        self.batch_depth -= 1;
        if self.batch_depth > 0 {
            return;
        }
        let need_persist = std::mem::take(&mut self.batch_dirty_persist);
        let need_state = std::mem::take(&mut self.batch_dirty_state);
        if need_persist {
            self.persist_best_effort_inner();
        }
        if need_state {
            self.rebuild_state_inner();
            self.emit_state_inner();
        }
    }
}

fn relay_connection_status(status: RelayStatus) -> &'static str {
    match status {
        RelayStatus::Connected => "connected",
        RelayStatus::Initialized | RelayStatus::Pending | RelayStatus::Connecting => "connecting",
        RelayStatus::Sleeping => "sleeping",
        RelayStatus::Disconnected | RelayStatus::Terminated => "offline",
        RelayStatus::Banned => "blocked",
    }
}

fn typing_indicator_is_active(
    indicator: &TypingIndicatorRecord,
    now: u64,
    latest_message_secs_by_chat: &BTreeMap<String, u64>,
) -> bool {
    if indicator.expires_at_secs <= now {
        return false;
    }
    let latest_message_secs = latest_message_secs_by_chat
        .get(&indicator.chat_id)
        .copied()
        .unwrap_or(0);
    indicator.last_event_secs > latest_message_secs
}
