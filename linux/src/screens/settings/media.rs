use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, PreferencesSnapshot};

use crate::app_manager::AppManager;

pub(super) struct MediaGroup {
    pub(super) group: adw::PreferencesGroup,
    enabled: adw::SwitchRow,
    fallback: adw::SwitchRow,
    url: adw::EntryRow,
    key: adw::EntryRow,
    salt: adw::EntryRow,
    saved: PreferencesSnapshot,
    updating: Rc<Cell<bool>>,
}

impl MediaGroup {
    pub(super) fn update(&mut self, prefs: &PreferencesSnapshot) {
        self.updating.set(true);
        self.enabled.set_active(prefs.image_proxy_enabled);
        self.fallback.set_active(prefs.image_proxy_fallback_enabled);
        for (entry, old, new) in [
            (
                &self.url,
                &self.saved.image_proxy_url,
                &prefs.image_proxy_url,
            ),
            (
                &self.key,
                &self.saved.image_proxy_key_hex,
                &prefs.image_proxy_key_hex,
            ),
            (
                &self.salt,
                &self.saved.image_proxy_salt_hex,
                &prefs.image_proxy_salt_hex,
            ),
        ] {
            // Preserve unsaved sibling fields when another setting is applied.
            // Keeping the actual editor also preserves its caret and focus.
            if entry.text().as_str() == old && old != new {
                entry.set_text(new);
            }
        }
        self.saved = prefs.clone();
        self.updating.set(false);
    }
}

pub(super) fn media_group(prefs: &PreferencesSnapshot, manager: &Rc<AppManager>) -> MediaGroup {
    let group = adw::PreferencesGroup::builder().title("Media").build();
    let updating = Rc::new(Cell::new(false));

    let enabled = adw::SwitchRow::builder().title("Image proxy").build();
    enabled.set_widget_name("settings-image-proxy-enabled");
    enabled.set_active(prefs.image_proxy_enabled);
    {
        let manager = manager.clone();
        let updating = updating.clone();
        enabled.connect_active_notify(move |row| {
            if updating.get() {
                return;
            }
            manager.dispatch(AppAction::SetImageProxyEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&enabled);

    let fallback = adw::SwitchRow::builder()
        .title("Load original images if the proxy fails")
        .subtitle("Image hosts may see your IP address.")
        .build();
    fallback.set_widget_name("settings-image-proxy-fallback");
    fallback.set_active(prefs.image_proxy_fallback_enabled);
    enabled
        .bind_property("active", &fallback, "sensitive")
        .sync_create()
        .build();
    {
        let manager = manager.clone();
        let updating = updating.clone();
        fallback.connect_active_notify(move |row| {
            if updating.get() {
                return;
            }
            manager.dispatch(AppAction::SetImageProxyFallbackEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&fallback);

    let url = adw::EntryRow::builder()
        .title("Proxy URL")
        .show_apply_button(true)
        .build();
    url.set_widget_name("settings-image-proxy-url");
    url.set_text(&prefs.image_proxy_url);
    let manager_for_apply = manager.clone();
    url.connect_apply(move |row| {
        manager_for_apply.dispatch(AppAction::SetImageProxyUrl {
            url: row.text().to_string(),
        });
    });
    group.add(&url);

    let key = adw::PasswordEntryRow::builder()
        .title("Proxy key")
        .show_apply_button(true)
        .build();
    key.set_widget_name("settings-image-proxy-key");
    key.set_text(&prefs.image_proxy_key_hex);
    let manager_for_key = manager.clone();
    key.connect_apply(move |row| {
        manager_for_key.dispatch(AppAction::SetImageProxyKeyHex {
            key_hex: row.text().to_string(),
        });
    });
    group.add(&key);

    let salt = adw::PasswordEntryRow::builder()
        .title("Proxy salt")
        .show_apply_button(true)
        .build();
    salt.set_widget_name("settings-image-proxy-salt");
    salt.set_text(&prefs.image_proxy_salt_hex);
    let manager_for_salt = manager.clone();
    salt.connect_apply(move |row| {
        manager_for_salt.dispatch(AppAction::SetImageProxySaltHex {
            salt_hex: row.text().to_string(),
        });
    });
    group.add(&salt);

    let reset = adw::ActionRow::builder()
        .title("Reset image proxy settings")
        .activatable(true)
        .build();
    {
        let manager = manager.clone();
        let url = url.downgrade();
        let key = key.downgrade();
        let salt = salt.downgrade();
        reset.connect_activated(move |_| {
            let defaults = PreferencesSnapshot::default();
            if let Some(url) = url.upgrade() {
                url.set_text(&defaults.image_proxy_url);
            }
            if let Some(key) = key.upgrade() {
                key.set_text(&defaults.image_proxy_key_hex);
            }
            if let Some(salt) = salt.upgrade() {
                salt.set_text(&defaults.image_proxy_salt_hex);
            }
            manager.dispatch(AppAction::ResetImageProxySettings);
        });
    }
    group.add(&reset);

    MediaGroup {
        group,
        enabled,
        fallback,
        url,
        key: key.upcast(),
        salt: salt.upcast(),
        saved: prefs.clone(),
        updating,
    }
}
