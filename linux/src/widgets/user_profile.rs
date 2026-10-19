use crate::app_manager::AppManager;
use adw::prelude::*;
use iris_chat_core::{AppState, PreferencesSnapshot, UserProfileSnapshot};

pub(crate) struct Binding {
    owner: String,
    avatar: Option<(gtk::glib::WeakRef<gtk::Box>, i32, bool)>,
    name: Option<gtk::glib::WeakRef<gtk::Label>>,
    last: Option<(UserProfileSnapshot, PreferencesSnapshot, Option<String>)>,
}

impl Binding {
    pub(crate) fn is_alive(&self) -> bool {
        self.avatar
            .as_ref()
            .is_some_and(|(root, _, _)| root.upgrade().is_some())
            || self
                .name
                .as_ref()
                .is_some_and(|name| name.upgrade().is_some())
    }
    pub(crate) fn refresh(&mut self, manager: &AppManager, state: &AppState) -> bool {
        let root = self.avatar.as_ref().and_then(|(root, _, _)| root.upgrade());
        let name = self.name.as_ref().and_then(|label| label.upgrade());
        if root.is_none() && name.is_none() {
            return false;
        }
        let profile = manager.user_profile(&self.owner);
        let account = state.account.as_ref().map(|a| a.public_key_hex.clone());
        let key = (profile.clone(), state.preferences.clone(), account);
        if self.last.as_ref() == Some(&key) {
            return true;
        }
        self.last = Some(key);
        if let Some(label) = name {
            let escaped = gtk::glib::markup_escape_text(&profile.name.name);
            label.set_markup(&if profile.name.is_fallback {
                format!("<i>{escaped}</i>")
            } else {
                escaped.to_string()
            });
        }
        if let (Some(root), Some((_, size, nearby))) = (root, self.avatar.as_ref()) {
            while let Some(child) = root.first_child() {
                root.remove(&child);
            }
            let image = adw::Avatar::new(*size, Some(&profile.name.name), true);
            if let Some(url) = profile.picture_url.as_deref() {
                super::image_cache::fetch_proxied_into_avatar(
                    &image,
                    url,
                    &state.preferences,
                    *size as u32,
                );
            }
            let overlay = super::social_badge::avatar(&image, profile.social_connection.as_ref());
            if *nearby {
                super::social_badge::add_nearby_badge(&overlay, *size, &self.owner, manager);
            }
            root.append(&overlay);
        }
        true
    }
}

/// All user metadata comes from the shared core by identity. Replacing the
/// inner image on profile changes also retires old asynchronous image loads.
pub fn avatar(manager: &AppManager, owner: &str, size: i32, nearby: bool) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    manager.track_user_profile(Binding {
        owner: owner.into(),
        avatar: Some((root.downgrade(), size, nearby)),
        name: None,
        last: None,
    });
    root
}

pub fn name(manager: &AppManager, owner: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    manager.track_user_profile(Binding {
        owner: owner.into(),
        avatar: None,
        name: Some(label.downgrade()),
        last: None,
    });
    label
}
