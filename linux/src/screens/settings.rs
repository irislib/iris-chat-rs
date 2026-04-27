use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState, PreferencesSnapshot, Screen};

use crate::app_manager::AppManager;
use crate::platform::clipboard;
use crate::widgets::qr;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    if let Some(account) = state.account.as_ref() {
        page.add(&profile_group(account, manager));
    }

    page.add(&messaging_group(&state.preferences, manager));
    page.add(&relays_group(&state.preferences, manager));
    page.add(&security_group(manager));
    page.add(&about_group(state));
    page.add(&support_group(manager));

    page.upcast()
}

fn support_group(manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Support").build();

    let row = adw::ActionRow::builder()
        .title("Copy support bundle")
        .subtitle("Copies a JSON snapshot of runtime state for debugging")
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name("edit-copy-symbolic");
    row.add_prefix(&icon);
    {
        let manager = manager.clone();
        row.connect_activated(move |_| {
            let json = manager.export_support_bundle_json();
            clipboard::copy(&json);
        });
    }
    group.add(&row);

    group
}

fn profile_group(
    account: &ndr_demo_core::AccountSnapshot,
    manager: &Rc<AppManager>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Profile").build();

    let avatar_row = adw::ActionRow::new();
    let avatar = adw::Avatar::new(56, Some(&account.display_name), true);
    avatar_row.add_prefix(&avatar);
    avatar_row.set_title(&account.display_name);
    avatar_row.set_subtitle(&short_npub(&account.npub));

    let change_pic = gtk::Button::with_label("Change photo");
    change_pic.add_css_class("flat");
    change_pic.set_valign(gtk::Align::Center);
    let manager_for_pic = manager.clone();
    change_pic.connect_clicked(move |btn| {
        let parent = btn
            .root()
            .and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder()
            .title("Choose profile picture")
            .build();
        let manager = manager_for_pic.clone();
        dialog.open(parent.as_ref(), gtk::gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            if let Some(path) = file.path() {
                manager.dispatch(AppAction::UploadProfilePicture {
                    file_path: path.to_string_lossy().to_string(),
                });
            }
        });
    });
    avatar_row.add_suffix(&change_pic);
    group.add(&avatar_row);

    let name_row = adw::EntryRow::builder().title("Display name").build();
    name_row.set_text(&account.display_name);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_valign(gtk::Align::Center);
    let manager_for_save = manager.clone();
    let row_for_save = name_row.clone();
    let picture_url = account.picture_url.clone();
    save.connect_clicked(move |_| {
        let value = row_for_save.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        manager_for_save.dispatch(AppAction::UpdateProfileMetadata {
            name: value,
            picture_url: picture_url.clone(),
        });
    });
    name_row.add_suffix(&save);

    let manager_for_apply = manager.clone();
    let picture_url = account.picture_url.clone();
    name_row.connect_apply(move |row| {
        let value = row.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        manager_for_apply.dispatch(AppAction::UpdateProfileMetadata {
            name: value,
            picture_url: picture_url.clone(),
        });
    });
    group.add(&name_row);

    let qr_row = adw::ActionRow::builder()
        .title("Show my QR")
        .subtitle("Share your npub for someone else to start a chat")
        .activatable(true)
        .build();
    let qr_icon = gtk::Image::from_icon_name("preferences-other-symbolic");
    qr_row.add_prefix(&qr_icon);
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("dim-label");
    qr_row.add_suffix(&chevron);
    let npub = account.npub.clone();
    let display_name = account.display_name.clone();
    qr_row.connect_activated(move |row| {
        let parent = row
            .root()
            .and_then(|r| r.downcast::<gtk::Window>().ok());
        present_qr_dialog(parent.as_ref(), &display_name, &npub);
    });
    group.add(&qr_row);

    let devices_row = adw::ActionRow::builder()
        .title("Manage devices")
        .activatable(true)
        .build();
    let dev_icon = gtk::Image::from_icon_name("computer-symbolic");
    devices_row.add_prefix(&dev_icon);
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("dim-label");
    devices_row.add_suffix(&chevron);
    {
        let manager = manager.clone();
        devices_row.connect_activated(move |_| {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::DeviceRoster,
            });
        });
    }
    group.add(&devices_row);

    group
}

fn present_qr_dialog(parent: Option<&gtk::Window>, name: &str, npub: &str) {
    let dialog = adw::Dialog::builder()
        .title(name)
        .content_width(360)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let header = gtk::Label::new(Some(name));
    header.add_css_class("title-2");
    content.append(&header);

    content.append(&qr::build(npub, 240));

    let label = gtk::Label::new(Some(npub));
    label.add_css_class("monospace");
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_max_width_chars(40);
    label.set_selectable(true);
    label.set_xalign(0.5);
    content.append(&label);

    let copy = gtk::Button::with_label("Copy");
    copy.add_css_class("pill");
    copy.add_css_class("suggested-action");
    let npub_owned = npub.to_string();
    copy.connect_clicked(move |_| clipboard::copy(&npub_owned));
    content.append(&copy);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn messaging_group(prefs: &PreferencesSnapshot, manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Messaging").build();

    let typing = adw::SwitchRow::builder().title("Typing indicators").build();
    typing.set_active(prefs.send_typing_indicators);
    {
        let manager = manager.clone();
        typing.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetTypingIndicatorsEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&typing);

    let receipts = adw::SwitchRow::builder().title("Read receipts").build();
    receipts.set_active(prefs.send_read_receipts);
    {
        let manager = manager.clone();
        receipts.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetReadReceiptsEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&receipts);

    let notifications = adw::SwitchRow::builder().title("Notifications").build();
    notifications.set_active(prefs.desktop_notifications_enabled);
    {
        let manager = manager.clone();
        notifications.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetDesktopNotificationsEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&notifications);

    group
}

fn relays_group(prefs: &PreferencesSnapshot, manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Relays").build();

    for url in &prefs.nostr_relay_urls {
        let row = adw::ActionRow::builder().title(url).build();
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove"));
        let manager_for_remove = manager.clone();
        let relay_url = url.clone();
        remove.connect_clicked(move |_| {
            manager_for_remove.dispatch(AppAction::RemoveNostrRelay {
                relay_url: relay_url.clone(),
            });
        });
        row.add_suffix(&remove);
        group.add(&row);
    }

    let add_row = adw::EntryRow::builder().title("Add relay").build();
    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.add_css_class("flat");
    add_button.set_valign(gtk::Align::Center);
    add_button.set_tooltip_text(Some("Add"));

    let manager_for_button = manager.clone();
    let row_for_button = add_row.clone();
    add_button.connect_clicked(move |_| {
        let value = row_for_button.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        manager_for_button.dispatch(AppAction::AddNostrRelay { relay_url: value });
        row_for_button.set_text("");
    });
    add_row.add_suffix(&add_button);

    let manager_for_apply = manager.clone();
    add_row.connect_apply(move |row| {
        let value = row.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        manager_for_apply.dispatch(AppAction::AddNostrRelay { relay_url: value });
        row.set_text("");
    });
    group.add(&add_row);

    let reset = adw::ActionRow::builder()
        .title("Reset to defaults")
        .activatable(true)
        .build();
    {
        let manager = manager.clone();
        reset.connect_activated(move |_| {
            manager.dispatch(AppAction::ResetNostrRelays);
        });
    }
    group.add(&reset);

    group
}

fn security_group(manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Security").build();

    let logout = adw::ActionRow::builder()
        .title("Sign out of this device")
        .subtitle("Clears local secrets and chat data")
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name("system-log-out-symbolic");
    icon.add_css_class("error");
    logout.add_prefix(&icon);
    logout.add_css_class("error");
    {
        let manager = manager.clone();
        logout.connect_activated(move |_| {
            manager.dispatch(AppAction::Logout);
        });
    }
    group.add(&logout);

    group
}

fn about_group(state: &AppState) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("About").build();

    let version = adw::ActionRow::builder()
        .title("Version")
        .subtitle(env!("CARGO_PKG_VERSION"))
        .build();
    group.add(&version);

    if let Some(net) = state.network_status.as_ref() {
        let status = adw::ActionRow::builder()
            .title("Network")
            .subtitle(format!(
                "{} · {} relays · {} events",
                if net.syncing { "syncing" } else { "idle" },
                net.relay_urls.len(),
                net.recent_event_count
            ))
            .build();
        group.add(&status);
    }

    group
}

fn short_npub(npub: &str) -> String {
    if npub.len() <= 16 {
        return npub.to_string();
    }
    format!("{}…{}", &npub[..10], &npub[npub.len() - 6..])
}
