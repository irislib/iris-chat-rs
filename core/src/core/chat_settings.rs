use super::*;

impl AppCore {
    pub(super) fn set_typing_indicators_enabled(&mut self, enabled: bool) {
        if self.preferences.send_typing_indicators == enabled {
            return;
        }
        self.preferences.send_typing_indicators = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_read_receipts_enabled(&mut self, enabled: bool) {
        if self.preferences.send_read_receipts == enabled {
            return;
        }
        self.preferences.send_read_receipts = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_chat_message_ttl(&mut self, chat_id: &str, ttl_seconds: Option<u64>) {
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        let previous_ttl = self
            .chat_message_ttl_seconds
            .get(&normalized_chat_id)
            .copied();
        let normalized_ttl = match ttl_seconds {
            Some(ttl_seconds) if ttl_seconds > 0 => {
                self.chat_message_ttl_seconds
                    .insert(normalized_chat_id.clone(), ttl_seconds);
                Some(ttl_seconds)
            }
            _ => {
                self.chat_message_ttl_seconds.remove(&normalized_chat_id);
                None
            }
        };
        if previous_ttl == normalized_ttl {
            return;
        }

        let actor = self
            .logged_in
            .as_ref()
            .map(|logged_in| self.owner_display_label(&logged_in.owner_pubkey.to_hex()))
            .unwrap_or_else(|| "You".to_string());
        self.push_system_notice(
            &normalized_chat_id,
            disappearing_timer_notice(&actor, normalized_ttl),
            unix_now().get(),
        );
        if is_group_chat_id(&normalized_chat_id) {
            let content = serde_json::json!({
                "type": "chat-settings",
                "v": 1,
                "messageTtlSeconds": normalized_ttl.unwrap_or(0),
            })
            .to_string();
            self.send_group_event(
                &normalized_chat_id,
                CHAT_SETTINGS_KIND,
                &content,
                Vec::new(),
                None,
            );
        } else if let (Some(owner_pubkey), Ok((_, peer))) = (
            self.logged_in
                .as_ref()
                .map(|logged_in| logged_in.owner_pubkey),
            parse_peer_input(&normalized_chat_id),
        ) {
            let ttl = normalized_ttl.unwrap_or(0);
            let now = unix_now();
            let ttl_setting = if ttl == 0 {
                pairwise_codec::ChatSettingsTtl::DisablePeerExpiration
            } else {
                pairwise_codec::ChatSettingsTtl::Seconds(ttl)
            };
            if let Ok(unsigned) = pairwise_codec::chat_settings_event(
                owner_pubkey,
                ttl_setting,
                now.get(),
                now.get().saturating_mul(1000),
            ) {
                self.send_protocol_engine_unsigned_event(
                    peer,
                    &normalized_chat_id,
                    unsigned,
                    "chat_settings",
                );
            }
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_chat_muted(&mut self, chat_id: &str, muted: bool) {
        let Some(normalized_chat_id) = self.normalize_local_chat_setting_id(chat_id) else {
            return;
        };

        let mut muted_chat_ids = self.preferences.muted_chat_ids.clone();
        muted_chat_ids.sort();
        muted_chat_ids.dedup();
        let had_muted = muted_chat_ids
            .iter()
            .any(|existing| existing == &normalized_chat_id);

        if muted == had_muted {
            if muted_chat_ids != self.preferences.muted_chat_ids {
                self.preferences.muted_chat_ids = muted_chat_ids;
                self.persist_best_effort();
            }
            return;
        }

        if muted {
            muted_chat_ids.push(normalized_chat_id.clone());
            muted_chat_ids.sort();
            muted_chat_ids.dedup();
        } else {
            muted_chat_ids.retain(|existing| existing != &normalized_chat_id);
        }
        self.preferences.muted_chat_ids = muted_chat_ids;
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn is_chat_muted(&self, chat_id: &str) -> bool {
        self.normalize_local_chat_setting_id(chat_id)
            .is_some_and(|normalized| {
                self.preferences
                    .muted_chat_ids
                    .iter()
                    .any(|chat_id| chat_id == &normalized)
            })
    }

    pub(super) fn set_chat_pinned(&mut self, chat_id: &str, pinned: bool) {
        let Some(normalized_chat_id) = self.normalize_local_chat_setting_id(chat_id) else {
            return;
        };

        let mut pinned_chat_ids = self.preferences.pinned_chat_ids.clone();
        pinned_chat_ids.sort();
        pinned_chat_ids.dedup();
        let had_pinned = pinned_chat_ids
            .iter()
            .any(|existing| existing == &normalized_chat_id);

        if pinned == had_pinned {
            if pinned_chat_ids != self.preferences.pinned_chat_ids {
                self.preferences.pinned_chat_ids = pinned_chat_ids;
                self.persist_best_effort();
            }
            return;
        }

        if pinned {
            pinned_chat_ids.push(normalized_chat_id.clone());
            pinned_chat_ids.sort();
            pinned_chat_ids.dedup();
        } else {
            pinned_chat_ids.retain(|existing| existing != &normalized_chat_id);
        }
        self.preferences.pinned_chat_ids = pinned_chat_ids;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn is_chat_pinned(&self, chat_id: &str) -> bool {
        self.normalize_local_chat_setting_id(chat_id)
            .is_some_and(|normalized| {
                self.preferences
                    .pinned_chat_ids
                    .iter()
                    .any(|chat_id| chat_id == &normalized)
            })
    }

    fn normalize_local_chat_setting_id(&self, chat_id: &str) -> Option<String> {
        let trimmed = chat_id.trim();
        if trimmed.is_empty() {
            return None;
        }
        if is_group_chat_id(trimmed) {
            return parse_group_id_from_chat_id(trimmed).map(|group_id| group_chat_id(&group_id));
        }
        parse_peer_input(trimmed)
            .ok()
            .map(|(normalized, _)| normalized)
    }

    pub(super) fn set_desktop_notifications_enabled(&mut self, enabled: bool) {
        if self.preferences.desktop_notifications_enabled == enabled {
            return;
        }
        self.preferences.desktop_notifications_enabled = enabled;
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_invite_acceptance_notifications_enabled(&mut self, enabled: bool) {
        if self.preferences.invite_acceptance_notifications_enabled == enabled {
            return;
        }
        self.preferences.invite_acceptance_notifications_enabled = enabled;
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_startup_at_login_enabled(&mut self, enabled: bool) {
        if self.preferences.startup_at_login_enabled == enabled {
            return;
        }
        self.preferences.startup_at_login_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_nearby_enabled(&mut self, enabled: bool) {
        let changed = self.preferences.nearby_enabled != enabled;
        self.preferences.nearby_enabled = enabled;
        if !changed {
            return;
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_nearby_mailbag_enabled(&mut self, enabled: bool) {
        if self.preferences.nearby_mailbag_enabled == enabled {
            return;
        }
        self.preferences.nearby_mailbag_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_nearby_show_in_chat_list(&mut self, enabled: bool) {
        if self.preferences.nearby_show_in_chat_list == enabled {
            return;
        }
        self.preferences.nearby_show_in_chat_list = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_nearby_bluetooth_enabled(&mut self, enabled: bool) {
        if self.preferences.nearby_bluetooth_enabled == enabled {
            return;
        }
        self.preferences.nearby_bluetooth_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_nearby_lan_enabled(&mut self, enabled: bool) {
        if self.preferences.nearby_lan_enabled == enabled {
            return;
        }
        self.preferences.nearby_lan_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_debug_logging_enabled(&mut self, enabled: bool) {
        if self.preferences.debug_logging_enabled == enabled {
            return;
        }
        self.preferences.debug_logging_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_accept_unknown_direct_messages(&mut self, enabled: bool) {
        if self.preferences.accept_unknown_direct_messages == enabled {
            return;
        }
        self.preferences.accept_unknown_direct_messages = enabled;
        // The toggle gates whether non-accepted peers stay in the
        // nostr/push subscription author set, so a flip has to push
        // a fresh subscription out to both layers.
        self.request_protocol_subscription_refresh();
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_user_blocked(&mut self, owner_pubkey_hex: &str, blocked: bool) {
        let normalized = owner_pubkey_hex.trim().to_lowercase();
        if normalized.is_empty() {
            return;
        }
        let already_blocked = self
            .preferences
            .blocked_owner_pubkeys
            .iter()
            .any(|hex| hex == &normalized);
        if blocked == already_blocked {
            return;
        }
        if blocked {
            self.preferences.blocked_owner_pubkeys.push(normalized);
            self.preferences.blocked_owner_pubkeys.sort();
            self.preferences.blocked_owner_pubkeys.dedup();
        } else {
            self.preferences
                .blocked_owner_pubkeys
                .retain(|hex| hex != &normalized);
        }
        self.request_protocol_subscription_refresh();
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn accept_message_request(&mut self, chat_id: &str) {
        let normalized = chat_id.trim().to_lowercase();
        if normalized.is_empty() {
            return;
        }
        // For group threads the peer to whitelist is the group's
        // creator (Signal-style: accepting means "I trust whoever
        // added me"). For direct threads the chat_id is already the
        // peer's owner pubkey.
        let target_owner_hex = if is_group_chat_id(&normalized) {
            let Some(group_id) = parse_group_id_from_chat_id(&normalized) else {
                return;
            };
            let Some(group) = self.groups.get(&group_id) else {
                return;
            };
            group.created_by.to_string()
        } else {
            normalized
        };
        if self
            .preferences
            .accepted_owner_pubkeys
            .iter()
            .any(|hex| hex == &target_owner_hex)
        {
            return;
        }
        self.preferences
            .accepted_owner_pubkeys
            .push(target_owner_hex);
        self.preferences.accepted_owner_pubkeys.sort();
        self.preferences.accepted_owner_pubkeys.dedup();
        // Accepting a request moves the peer into the
        // "subscribed-when-toggle-is-off" set, so push needs to learn
        // about them. Nostr already has the session, but refreshing is
        // cheap and keeps both layers in sync.
        self.request_protocol_subscription_refresh();
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn add_nostr_relay(&mut self, relay_url: &str) {
        let normalized = match normalize_nostr_relay_url(relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        if self.preferences.nostr_relay_urls.contains(&normalized) {
            return self.reject_relay_setting("Relay already exists.".to_string());
        }

        let mut next = self.preferences.nostr_relay_urls.clone();
        next.push(normalized);
        self.apply_nostr_relay_urls(next);
    }

    pub(super) fn update_nostr_relay(&mut self, old_relay_url: &str, new_relay_url: &str) {
        let old_normalized = match normalize_nostr_relay_url(old_relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        let new_normalized = match normalize_nostr_relay_url(new_relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        let Some(index) = self
            .preferences
            .nostr_relay_urls
            .iter()
            .position(|relay| relay == &old_normalized)
        else {
            return self.reject_relay_setting("Relay not found.".to_string());
        };
        if old_normalized != new_normalized
            && self.preferences.nostr_relay_urls.contains(&new_normalized)
        {
            return self.reject_relay_setting("Relay already exists.".to_string());
        }

        let mut next = self.preferences.nostr_relay_urls.clone();
        if let Some(relay) = next.get_mut(index) {
            *relay = new_normalized;
        }
        self.apply_nostr_relay_urls(next);
    }

    pub(super) fn remove_nostr_relay(&mut self, relay_url: &str) {
        let normalized = match normalize_nostr_relay_url(relay_url) {
            Ok(url) => url,
            Err(message) => return self.reject_relay_setting(message),
        };
        let Some(index) = self
            .preferences
            .nostr_relay_urls
            .iter()
            .position(|relay| relay == &normalized)
        else {
            return self.reject_relay_setting("Relay not found.".to_string());
        };

        let mut next = self.preferences.nostr_relay_urls.clone();
        next.remove(index);
        self.apply_nostr_relay_urls(next);
    }

    pub(super) fn set_nostr_relays(&mut self, relay_urls: &[String]) {
        self.apply_nostr_relay_urls(relay_urls.to_vec());
    }

    pub(super) fn reset_nostr_relays(&mut self) {
        self.apply_nostr_relay_urls(configured_relays());
        self.state.toast = Some("Relays reset to defaults.".to_string());
        self.emit_state();
    }

    pub(super) fn set_image_proxy_enabled(&mut self, enabled: bool) {
        if self.preferences.image_proxy_enabled == enabled {
            return;
        }
        self.preferences.image_proxy_enabled = enabled;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_image_proxy_url(&mut self, url: &str) {
        let normalized = normalized_setting(url, crate::image_proxy::DEFAULT_IMAGE_PROXY_URL);
        if self.preferences.image_proxy_url == normalized {
            return;
        }
        self.preferences.image_proxy_url = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_image_proxy_key_hex(&mut self, key_hex: &str) {
        let normalized = normalized_setting(
            &key_hex.to_ascii_lowercase(),
            crate::image_proxy::DEFAULT_IMAGE_PROXY_KEY_HEX,
        );
        if self.preferences.image_proxy_key_hex == normalized {
            return;
        }
        self.preferences.image_proxy_key_hex = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_image_proxy_salt_hex(&mut self, salt_hex: &str) {
        let normalized = normalized_setting(
            &salt_hex.to_ascii_lowercase(),
            crate::image_proxy::DEFAULT_IMAGE_PROXY_SALT_HEX,
        );
        if self.preferences.image_proxy_salt_hex == normalized {
            return;
        }
        self.preferences.image_proxy_salt_hex = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn set_mobile_push_server_url(&mut self, url: &str) {
        let normalized = url.trim().to_string();
        if self.preferences.mobile_push_server_url == normalized {
            return;
        }
        self.preferences.mobile_push_server_url = normalized;
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn reset_mobile_push_server_url(&mut self) {
        if self.preferences.mobile_push_server_url.is_empty() {
            return;
        }
        self.preferences.mobile_push_server_url.clear();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn reset_image_proxy_settings(&mut self) {
        self.preferences.image_proxy_enabled = true;
        self.preferences.image_proxy_url = crate::image_proxy::DEFAULT_IMAGE_PROXY_URL.to_string();
        self.preferences.image_proxy_key_hex =
            crate::image_proxy::DEFAULT_IMAGE_PROXY_KEY_HEX.to_string();
        self.preferences.image_proxy_salt_hex =
            crate::image_proxy::DEFAULT_IMAGE_PROXY_SALT_HEX.to_string();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn apply_nostr_relay_urls(&mut self, relay_urls: Vec<String>) {
        let normalized = normalize_nostr_relay_urls(&relay_urls);
        if self.preferences.nostr_relay_urls == normalized {
            if self.logged_in.is_some() {
                self.publish_local_app_keys_snapshot();
                self.schedule_session_connect();
                self.request_protocol_subscription_refresh_forced();
                self.fetch_recent_protocol_state();
                self.retry_protocol_engine_pending_outbound("relays_refreshed");
                self.retry_pending_relay_publishes("relays_refreshed");
            }
            return;
        }

        self.preferences.nostr_relay_urls = normalized;
        let next_relay_urls = relay_urls_from_strings(&self.preferences.nostr_relay_urls);
        let should_refresh = if let Some(logged_in) = self.logged_in.as_mut() {
            let client = logged_in.client.clone();
            let previous_relay_urls = logged_in.relay_urls.clone();
            logged_in.relay_urls = next_relay_urls.clone();
            self.runtime.spawn(async move {
                sync_session_relays(&client, &previous_relay_urls, &next_relay_urls).await;
            });
            true
        } else {
            false
        };

        self.state.preferences = self.preferences.clone();
        if let Some(network_status) = self.state.network_status.as_mut() {
            network_status.relay_urls = self.preferences.nostr_relay_urls.clone();
            network_status.relay_connections = self
                .preferences
                .nostr_relay_urls
                .iter()
                .map(|url| RelayConnectionSnapshot {
                    url: url.clone(),
                    status: "connecting".to_string(),
                })
                .collect();
            network_status.connected_relay_count = 0;
        }
        self.persist_best_effort();
        self.emit_state();

        if should_refresh {
            let configured_relays = self
                .preferences
                .nostr_relay_urls
                .iter()
                .filter_map(|url| normalize_nostr_relay_url(url).ok())
                .collect::<HashSet<_>>();
            self.relay_status_watch_urls
                .retain(|url| configured_relays.contains(url));
            self.relay_status_by_url
                .retain(|url, _| configured_relays.contains(url));
            self.schedule_session_connect();
            self.request_protocol_subscription_refresh_forced();
            self.publish_local_app_keys_snapshot();
            self.fetch_recent_protocol_state();
            self.retry_protocol_engine_pending_outbound("relays_changed");
            self.retry_pending_relay_publishes("relays_changed");
        }
    }

    fn reject_relay_setting(&mut self, message: String) {
        self.state.toast = Some(message);
        self.emit_state();
    }

    pub(super) fn apply_chat_settings_control(
        &mut self,
        chat_id: &str,
        actor: &str,
        ttl_seconds: Option<u64>,
        created_at_secs: u64,
    ) {
        let Some(normalized_chat_id) = self.normalize_chat_id(chat_id) else {
            return;
        };
        match ttl_seconds {
            Some(ttl_seconds) if ttl_seconds > 0 => {
                self.chat_message_ttl_seconds
                    .insert(normalized_chat_id.clone(), ttl_seconds);
            }
            _ => {
                self.chat_message_ttl_seconds.remove(&normalized_chat_id);
            }
        }
        self.push_system_notice(
            &normalized_chat_id,
            disappearing_timer_notice(actor, ttl_seconds),
            created_at_secs,
        );
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }
}

fn disappearing_timer_notice(actor: &str, ttl_seconds: Option<u64>) -> String {
    format!(
        "{actor} set disappearing messages timer to {}",
        disappearing_timer_label(ttl_seconds)
    )
}

fn disappearing_timer_label(ttl_seconds: Option<u64>) -> String {
    match ttl_seconds {
        None | Some(0) => "Off".to_string(),
        Some(300) => "5 minutes".to_string(),
        Some(3600) => "1 hour".to_string(),
        Some(86_400) => "24 hours".to_string(),
        Some(604_800) => "1 week".to_string(),
        Some(2_592_000) => "1 month".to_string(),
        Some(7_776_000) => "3 months".to_string(),
        Some(seconds) if seconds % 86_400 == 0 => {
            let days = seconds / 86_400;
            format!("{days} days")
        }
        Some(seconds) if seconds % 3600 == 0 => {
            let hours = seconds / 3600;
            if hours == 1 {
                "1 hour".to_string()
            } else {
                format!("{hours} hours")
            }
        }
        Some(seconds) if seconds % 60 == 0 => {
            let minutes = seconds / 60;
            if minutes == 1 {
                "1 minute".to_string()
            } else {
                format!("{minutes} minutes")
            }
        }
        Some(seconds) => format!("{seconds} seconds"),
    }
}

fn normalized_setting(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}
