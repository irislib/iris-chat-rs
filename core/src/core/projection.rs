use super::*;

impl AppCore {
    pub(super) fn rebuild_state(&mut self) {
        self.state.account = self.build_account_snapshot();
        self.state.device_roster = self.build_device_roster_snapshot();
        self.state.network_status = Some(self.build_network_status_snapshot());
        self.state.public_invite = self.build_public_invite_snapshot();
        self.state.mobile_push = self.build_mobile_push_sync_snapshot();
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
                let display_name = group_snapshot
                    .as_ref()
                    .map(|group| group.name.clone())
                    .unwrap_or_else(|| self.owner_display_label(&thread.chat_id));
                let subtitle = group_snapshot
                    .as_ref()
                    .map(|group| format!("{} members", group.members.len()))
                    .or_else(|| self.owner_secondary_identifier(&thread.chat_id));
                let member_count = group_snapshot
                    .as_ref()
                    .map(|group| group.members.len() as u64)
                    .unwrap_or(0);
                ChatThreadSnapshot {
                    chat_id: thread.chat_id.clone(),
                    kind: thread_kind,
                    display_name,
                    subtitle,
                    member_count,
                    last_message_preview: last_message.map(message_preview),
                    last_message_at_secs: last_message.map(|message| message.created_at_secs),
                    last_message_is_outgoing: last_message.map(|message| message.is_outgoing),
                    last_message_delivery: last_message.map(|message| message.delivery.clone()),
                    unread_count: thread.unread_count,
                    is_typing: self.thread_has_typing_indicator(&thread.chat_id),
                }
            })
            .collect();

        self.state.current_chat = self
            .active_chat_id
            .as_ref()
            .and_then(|chat_id| self.threads.get(chat_id))
            .map(|thread| {
                let group_snapshot = self.group_snapshot_for_chat_id(&thread.chat_id);
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
                    group_id: group_snapshot.as_ref().map(|group| group.group_id.clone()),
                    member_count: group_snapshot
                        .as_ref()
                        .map(|group| group.members.len() as u64)
                        .unwrap_or(0),
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
    }

    pub(super) fn prune_expired_typing_indicators(&mut self) {
        let now = unix_now().get();
        self.typing_indicators
            .retain(|_, indicator| indicator.expires_at_secs > now);
    }

    pub(super) fn thread_has_typing_indicator(&self, chat_id: &str) -> bool {
        let now = unix_now().get();
        self.typing_indicators
            .values()
            .any(|indicator| indicator.chat_id == chat_id && indicator.expires_at_secs > now)
    }

    pub(super) fn typing_indicator_snapshots(&self, chat_id: &str) -> Vec<TypingIndicatorSnapshot> {
        let now = unix_now().get();
        let mut indicators = self
            .typing_indicators
            .values()
            .filter(|indicator| indicator.chat_id == chat_id && indicator.expires_at_secs > now)
            .map(|indicator| TypingIndicatorSnapshot {
                chat_id: indicator.chat_id.clone(),
                display_name: self.owner_display_label(&indicator.author_owner_hex),
                expires_at_secs: indicator.expires_at_secs,
            })
            .collect::<Vec<_>>();
        indicators.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        indicators
    }

    pub(super) fn build_account_snapshot(&self) -> Option<AccountSnapshot> {
        let logged_in = self.logged_in.as_ref()?;
        let owner_public_key_hex = logged_in.owner_pubkey.to_string();
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

        if let Some(user) = logged_in
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == logged_in.owner_pubkey)
        {
            if let Some(roster) = user.roster.as_ref() {
                for authorized_device in roster.devices() {
                    let device_pubkey_hex = authorized_device.device_pubkey.to_string();
                    entries
                        .entry(device_pubkey_hex.clone())
                        .or_insert(DeviceEntrySnapshot {
                            device_pubkey_hex: device_pubkey_hex.clone(),
                            device_npub: device_npub(&device_pubkey_hex)
                                .unwrap_or_else(|| device_pubkey_hex.clone()),
                            is_current_device: device_pubkey_hex == current_device_pubkey_hex,
                            is_authorized: true,
                            is_stale: false,
                            last_activity_secs: None,
                        });
                }
            }

            for device in user.devices {
                let device_pubkey_hex = device.device_pubkey.to_string();
                let entry =
                    entries
                        .entry(device_pubkey_hex.clone())
                        .or_insert(DeviceEntrySnapshot {
                            device_pubkey_hex: device_pubkey_hex.clone(),
                            device_npub: device_npub(&device_pubkey_hex)
                                .unwrap_or_else(|| device_pubkey_hex.clone()),
                            is_current_device: device_pubkey_hex == current_device_pubkey_hex,
                            is_authorized: device.authorized,
                            is_stale: device.is_stale,
                            last_activity_secs: device.last_activity.map(UnixSeconds::get),
                        });
                entry.is_authorized = device.authorized;
                entry.is_stale = device.is_stale;
                entry.last_activity_secs = device.last_activity.map(UnixSeconds::get);
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
        let recent_event_count = self.debug_event_counters.roster_events
            + self.debug_event_counters.invite_events
            + self.debug_event_counters.invite_response_events
            + self.debug_event_counters.message_events
            + self.debug_event_counters.other_events;
        let last_debug = self.debug_log.back();

        NetworkStatusSnapshot {
            relay_set_id: RELAY_SET_ID.to_string(),
            relay_urls: self.preferences.nostr_relay_urls.clone(),
            syncing: self.state.busy.syncing_network,
            pending_outbound_count: self.pending_outbound.len() as u64,
            pending_group_control_count: self.pending_group_controls.len() as u64,
            recent_event_count,
            recent_log_count: self.debug_log.len() as u64,
            last_debug_category: last_debug.map(|entry| entry.category.clone()),
            last_debug_detail: last_debug.map(|entry| entry.detail.clone()),
        }
    }

    pub(super) fn build_public_invite_snapshot(&self) -> Option<PublicInviteSnapshot> {
        let invite = self
            .logged_in
            .as_ref()?
            .session_manager
            .snapshot()
            .local_invite?;
        let url = codec::invite_url(&invite, CHAT_INVITE_ROOT_URL).ok()?;
        Some(PublicInviteSnapshot { url })
    }

    pub(super) fn group_snapshot_for_chat_id(&self, chat_id: &str) -> Option<GroupSnapshot> {
        let group_id = parse_group_id_from_chat_id(chat_id)?;
        self.logged_in.as_ref()?.group_manager.group(&group_id)
    }

    pub(super) fn build_group_details_snapshot(
        &self,
        group_id: &str,
    ) -> Option<GroupDetailsSnapshot> {
        let logged_in = self.logged_in.as_ref()?;
        let group = logged_in.group_manager.group(group_id)?;
        let local_owner = logged_in.owner_pubkey;
        let mut members = group
            .members
            .iter()
            .map(|owner| {
                let owner_hex = owner.to_string();
                GroupMemberSnapshot {
                    owner_pubkey_hex: owner_hex.clone(),
                    display_name: self.owner_display_label(&owner_hex),
                    npub: owner_npub_from_owner(*owner).unwrap_or_else(|| owner_hex.clone()),
                    is_admin: group.admins.iter().any(|admin| admin == owner),
                    is_creator: group.created_by == *owner,
                    is_local_owner: *owner == local_owner,
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

        Some(GroupDetailsSnapshot {
            group_id: group.group_id,
            name: group.name,
            created_by_display_name: self.owner_display_label(&group.created_by.to_string()),
            created_by_npub: owner_npub_from_owner(group.created_by)
                .unwrap_or_else(|| group.created_by.to_string()),
            can_manage: group.admins.iter().any(|admin| admin == &local_owner),
            revision: group.revision,
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
                    .map(|logged_in| logged_in.owner_pubkey.to_string())
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
        self.state.rev = self.state.rev.saturating_add(1);
        let snapshot = self.state.clone();
        match self.shared_state.write() {
            Ok(mut slot) => *slot = snapshot.clone(),
            Err(poison) => *poison.into_inner() = snapshot.clone(),
        }
        let _ = self.update_tx.send(AppUpdate::FullState(snapshot));
    }
}
