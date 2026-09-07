use super::{AppStore, PersistedPreferences};
use crate::state::PreferencesSnapshot;
use rusqlite::{params, OptionalExtension, Transaction};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

impl AppStore {
    pub(crate) fn load_preferences_snapshot(
        &mut self,
    ) -> anyhow::Result<Option<PersistedPreferences>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        load_preferences(&conn)
    }

    pub(crate) fn save_preferences(
        &mut self,
        preferences: &PreferencesSnapshot,
    ) -> anyhow::Result<()> {
        let preferences_hash = hash_preferences(preferences);
        if self.cache.preferences == Some(preferences_hash) {
            return Ok(());
        }

        {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            write_preferences(&tx, preferences)?;
            tx.commit()?;
        }

        self.cache.preferences = Some(preferences_hash);
        Ok(())
    }
}

pub(super) fn hash_preferences(preferences: &PreferencesSnapshot) -> u64 {
    let mut hasher = DefaultHasher::new();
    preferences.send_typing_indicators.hash(&mut hasher);
    preferences.send_read_receipts.hash(&mut hasher);
    preferences.desktop_notifications_enabled.hash(&mut hasher);
    preferences
        .invite_acceptance_notifications_enabled
        .hash(&mut hasher);
    preferences.startup_at_login_enabled.hash(&mut hasher);
    preferences.nearby_enabled.hash(&mut hasher);
    preferences.nearby_bluetooth_enabled.hash(&mut hasher);
    preferences.nearby_lan_enabled.hash(&mut hasher);
    preferences.nostr_relay_urls.hash(&mut hasher);
    preferences.image_proxy_enabled.hash(&mut hasher);
    preferences.image_proxy_fallback_enabled.hash(&mut hasher);
    preferences.image_proxy_url.hash(&mut hasher);
    preferences.image_proxy_key_hex.hash(&mut hasher);
    preferences.image_proxy_salt_hex.hash(&mut hasher);
    preferences.mobile_push_server_url.hash(&mut hasher);
    preferences.muted_chat_ids.hash(&mut hasher);
    preferences.pinned_chat_ids.hash(&mut hasher);
    preferences.debug_logging_enabled.hash(&mut hasher);
    preferences.accept_unknown_direct_messages.hash(&mut hasher);
    preferences.blocked_owner_pubkeys.hash(&mut hasher);
    preferences.accepted_owner_pubkeys.hash(&mut hasher);
    preferences.nearby_mailbag_enabled.hash(&mut hasher);
    preferences.nearby_show_in_chat_list.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn load_preferences(
    conn: &rusqlite::Connection,
) -> anyhow::Result<Option<PersistedPreferences>> {
    let row = conn
        .query_row(
            "SELECT send_typing_indicators, send_read_receipts, desktop_notifications_enabled,
                    invite_acceptance_notifications_enabled,
                    startup_at_login_enabled, nearby_bluetooth_enabled, nearby_lan_enabled,
                    nostr_relay_urls_json, image_proxy_enabled,
                    image_proxy_url, image_proxy_key_hex, image_proxy_salt_hex,
                    mobile_push_server_url, muted_chat_ids_json, pinned_chat_ids_json,
                    debug_logging_enabled, accept_unknown_direct_messages,
                    nearby_enabled,
                    blocked_owner_pubkeys_json, accepted_owner_pubkeys_json,
                    nearby_mailbag_enabled, nearby_show_in_chat_list, image_proxy_fallback_enabled
             FROM preferences WHERE id = 1",
            [],
            |row| {
                Ok(PersistedPreferences {
                    send_typing_indicators: row.get::<_, i64>(0)? != 0,
                    send_read_receipts: row.get::<_, i64>(1)? != 0,
                    desktop_notifications_enabled: row.get::<_, i64>(2)? != 0,
                    invite_acceptance_notifications_enabled: row.get::<_, i64>(3)? != 0,
                    startup_at_login_enabled: row.get::<_, i64>(4)? != 0,
                    nearby_bluetooth_enabled: row.get::<_, i64>(5)? != 0,
                    nearby_lan_enabled: row.get::<_, i64>(6)? != 0,
                    nostr_relay_urls: serde_json::from_str(&row.get::<_, String>(7)?)
                        .unwrap_or_default(),
                    image_proxy_enabled: row.get::<_, i64>(8)? != 0,
                    image_proxy_url: row.get::<_, String>(9)?,
                    image_proxy_key_hex: row.get::<_, String>(10)?,
                    image_proxy_salt_hex: row.get::<_, String>(11)?,
                    mobile_push_server_url: row.get::<_, String>(12)?,
                    muted_chat_ids: serde_json::from_str(&row.get::<_, String>(13)?)
                        .unwrap_or_default(),
                    pinned_chat_ids: serde_json::from_str(&row.get::<_, String>(14)?)
                        .unwrap_or_default(),
                    debug_logging_enabled: row.get::<_, i64>(15)? != 0,
                    accept_unknown_direct_messages: row.get::<_, i64>(16)? != 0,
                    nearby_enabled: row.get::<_, i64>(17)? != 0,
                    blocked_owner_pubkeys: serde_json::from_str(&row.get::<_, String>(18)?)
                        .unwrap_or_default(),
                    accepted_owner_pubkeys: serde_json::from_str(&row.get::<_, String>(19)?)
                        .unwrap_or_default(),
                    nearby_mailbag_enabled: row.get::<_, i64>(20)? != 0,
                    nearby_show_in_chat_list: row.get::<_, i64>(21)? != 0,
                    image_proxy_fallback_enabled: row.get::<_, i64>(22)? != 0,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub(super) fn write_preferences(
    tx: &Transaction,
    preferences: &PreferencesSnapshot,
) -> anyhow::Result<()> {
    let nostr_relay_urls_json = serde_json::to_string(&preferences.nostr_relay_urls)?;
    tx.execute(
        "INSERT INTO preferences (
            id, send_typing_indicators, send_read_receipts, desktop_notifications_enabled,
            invite_acceptance_notifications_enabled, startup_at_login_enabled,
            nearby_bluetooth_enabled, nearby_lan_enabled, nostr_relay_urls_json, image_proxy_enabled,
            image_proxy_url, image_proxy_key_hex, image_proxy_salt_hex,
            mobile_push_server_url, muted_chat_ids_json, pinned_chat_ids_json,
            debug_logging_enabled, accept_unknown_direct_messages, nearby_enabled,
            blocked_owner_pubkeys_json, accepted_owner_pubkeys_json,
            nearby_mailbag_enabled, nearby_show_in_chat_list, image_proxy_fallback_enabled
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
         ON CONFLICT(id) DO UPDATE SET
            send_typing_indicators = excluded.send_typing_indicators,
            send_read_receipts = excluded.send_read_receipts,
            desktop_notifications_enabled = excluded.desktop_notifications_enabled,
            invite_acceptance_notifications_enabled = excluded.invite_acceptance_notifications_enabled,
            startup_at_login_enabled = excluded.startup_at_login_enabled,
            nearby_bluetooth_enabled = excluded.nearby_bluetooth_enabled,
            nearby_lan_enabled = excluded.nearby_lan_enabled,
            nostr_relay_urls_json = excluded.nostr_relay_urls_json,
            image_proxy_enabled = excluded.image_proxy_enabled,
            image_proxy_fallback_enabled = excluded.image_proxy_fallback_enabled,
            image_proxy_url = excluded.image_proxy_url,
            image_proxy_key_hex = excluded.image_proxy_key_hex,
            image_proxy_salt_hex = excluded.image_proxy_salt_hex,
            mobile_push_server_url = excluded.mobile_push_server_url,
            muted_chat_ids_json = excluded.muted_chat_ids_json,
            pinned_chat_ids_json = excluded.pinned_chat_ids_json,
            debug_logging_enabled = excluded.debug_logging_enabled,
            accept_unknown_direct_messages = excluded.accept_unknown_direct_messages,
            nearby_enabled = excluded.nearby_enabled,
            blocked_owner_pubkeys_json = excluded.blocked_owner_pubkeys_json,
            accepted_owner_pubkeys_json = excluded.accepted_owner_pubkeys_json,
            nearby_mailbag_enabled = excluded.nearby_mailbag_enabled,
            nearby_show_in_chat_list = excluded.nearby_show_in_chat_list",
        params![
            preferences.send_typing_indicators as i64,
            preferences.send_read_receipts as i64,
            preferences.desktop_notifications_enabled as i64,
            preferences.invite_acceptance_notifications_enabled as i64,
            preferences.startup_at_login_enabled as i64,
            preferences.nearby_bluetooth_enabled as i64,
            preferences.nearby_lan_enabled as i64,
            nostr_relay_urls_json,
            preferences.image_proxy_enabled as i64,
            preferences.image_proxy_url,
            preferences.image_proxy_key_hex,
            preferences.image_proxy_salt_hex,
            preferences.mobile_push_server_url,
            serde_json::to_string(&preferences.muted_chat_ids)?,
            serde_json::to_string(&preferences.pinned_chat_ids)?,
            preferences.debug_logging_enabled as i64,
            preferences.accept_unknown_direct_messages as i64,
            preferences.nearby_enabled as i64,
            serde_json::to_string(&preferences.blocked_owner_pubkeys)?,
            serde_json::to_string(&preferences.accepted_owner_pubkeys)?,
            preferences.nearby_mailbag_enabled as i64,
            preferences.nearby_show_in_chat_list as i64,
            preferences.image_proxy_fallback_enabled as i64,
        ],
    )?;
    Ok(())
}
