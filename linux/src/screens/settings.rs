use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState, PreferencesSnapshot};

use crate::app_manager::AppManager;
use crate::platform::clipboard;
use crate::platform::startup;
use crate::screens::{chat_input_action, confirm_delete_app_data, device_roster, scan_qr_button};
use crate::widgets::{image_cache, qr};

const IRIS_SOURCE_URL: &str =
    "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs";

fn iris_chat_profile_url(npub: &str) -> String {
    format!("https://chat.iris.to/#/{npub}")
}

#[derive(Clone, Copy)]
enum SettingsPage {
    Profile,
    Devices,
    Messaging,
    Notifications,
    Media,
    Nearby,
    MessageServers,
    Updates,
    About,
    Support,
    AccountData,
}

impl SettingsPage {
    fn id(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Devices => "devices",
            Self::Messaging => "messaging",
            Self::Notifications => "notifications",
            Self::Media => "media",
            Self::Nearby => "nearby",
            Self::MessageServers => "message-servers",
            Self::Updates => "updates",
            Self::About => "about",
            Self::Support => "support",
            Self::AccountData => "account-data",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Profile => "Profile",
            Self::Devices => "Devices",
            Self::Messaging => "Messaging",
            Self::Notifications => "Notifications",
            Self::Media => "Media",
            Self::Nearby => "Nearby",
            Self::MessageServers => "Message servers",
            Self::Updates => "Updates",
            Self::About => "About",
            Self::Support => "Support",
            Self::AccountData => "Account data",
        }
    }

    fn icon_name(self) -> &'static str {
        match self {
            Self::Profile => "avatar-default-symbolic",
            Self::Devices => "computer-symbolic",
            Self::Messaging => "mail-message-new-symbolic",
            Self::Notifications => "preferences-system-notifications-symbolic",
            Self::Media => "image-x-generic-symbolic",
            Self::Nearby => "network-wireless-symbolic",
            Self::MessageServers => "network-server-symbolic",
            Self::Updates => "software-update-available-symbolic",
            Self::About => "help-about-symbolic",
            Self::Support => "tools-symbolic",
            Self::AccountData => "user-trash-symbolic",
        }
    }
}

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    if let Some(account) = state.account.as_ref() {
        stack.add_named(
            &settings_detail_page(vec![profile_group(account, &state.preferences, manager)]),
            Some(SettingsPage::Profile.id()),
        );
    }

    stack.add_named(
        &device_roster::content(state, manager),
        Some(SettingsPage::Devices.id()),
    );

    stack.add_named(
        &settings_detail_page(vec![messaging_group(&state.preferences, manager)]),
        Some(SettingsPage::Messaging.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![notifications_group(&state.preferences, manager)]),
        Some(SettingsPage::Notifications.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![media_group(&state.preferences, manager)]),
        Some(SettingsPage::Media.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![nearby_group(&state.preferences, manager)]),
        Some(SettingsPage::Nearby.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![relays_group(&state.preferences, manager)]),
        Some(SettingsPage::MessageServers.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![updates_group()]),
        Some(SettingsPage::Updates.id()),
    );
    let mut about_groups = Vec::new();
    if iris_chat_core::is_trusted_test_build() {
        about_groups.push(trusted_build_group());
    }
    about_groups.push(about_group(state));
    stack.add_named(
        &settings_detail_page(about_groups),
        Some(SettingsPage::About.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![support_group(manager)]),
        Some(SettingsPage::Support.id()),
    );
    stack.add_named(
        &settings_detail_page(vec![account_data_group(manager)]),
        Some(SettingsPage::AccountData.id()),
    );

    let sidebar = settings_menu(state, &stack);
    let sidebar_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(300)
        .child(&sidebar)
        .build();
    sidebar_scroll.set_width_request(320);
    root.append(&sidebar_scroll);

    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    root.append(&separator);

    let detail_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&stack)
        .build();
    root.append(&detail_scroll);

    let default_page = if state.account.is_some() {
        SettingsPage::Profile
    } else {
        SettingsPage::Messaging
    };
    stack.set_visible_child_name(default_page.id());

    root.upcast()
}

fn settings_detail_page(groups: Vec<adw::PreferencesGroup>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_margin_top(12);
    page.set_margin_bottom(24);
    page.set_margin_start(12);
    page.set_margin_end(24);
    for group in groups {
        page.add(&group);
    }
    page
}

fn settings_menu(state: &AppState, stack: &gtk::Stack) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_margin_top(12);
    page.set_margin_bottom(24);
    page.set_margin_start(12);
    page.set_margin_end(12);

    if let Some(account) = state.account.as_ref() {
        let group = adw::PreferencesGroup::new();
        let title = if account.display_name.is_empty() {
            "Profile"
        } else {
            &account.display_name
        };
        group.add(&settings_menu_row(
            SettingsPage::Profile,
            title,
            Some("My profile"),
            stack,
        ));
        page.add(&group);
    }

    let primary = adw::PreferencesGroup::new();
    for settings_page in [
        SettingsPage::Notifications,
        SettingsPage::Messaging,
        SettingsPage::Nearby,
        SettingsPage::Devices,
    ] {
        primary.add(&settings_menu_row(
            settings_page,
            settings_page.title(),
            None,
            stack,
        ));
    }
    page.add(&primary);

    let secondary = adw::PreferencesGroup::new();
    for settings_page in [
        SettingsPage::Updates,
        SettingsPage::Support,
        SettingsPage::About,
        SettingsPage::AccountData,
    ] {
        secondary.add(&settings_menu_row(
            settings_page,
            settings_page.title(),
            None,
            stack,
        ));
    }
    page.add(&secondary);

    let advanced = adw::PreferencesGroup::builder().title("Advanced").build();
    for settings_page in [SettingsPage::Media, SettingsPage::MessageServers] {
        advanced.add(&settings_menu_row(
            settings_page,
            settings_page.title(),
            None,
            stack,
        ));
    }
    page.add(&advanced);

    page
}

fn settings_menu_row(
    page: SettingsPage,
    title: &str,
    subtitle: Option<&str>,
    stack: &gtk::Stack,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle.unwrap_or_default())
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name(page.icon_name());
    row.add_prefix(&icon);
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("dim-label");
    row.add_suffix(&chevron);
    let stack = stack.clone();
    row.connect_activated(move |_| {
        stack.set_visible_child_name(page.id());
    });
    row
}

fn nearby_group(prefs: &PreferencesSnapshot, manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Nearby").build();

    let master = adw::SwitchRow::builder().title("Nearby").build();
    master.set_active(prefs.nearby_enabled);
    {
        let manager = manager.clone();
        master.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetNearbyEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&master);

    if prefs.nearby_enabled {
        let lan = adw::SwitchRow::builder().title("Wi-Fi").build();
        lan.set_active(prefs.nearby_lan_enabled);
        {
            let manager = manager.clone();
            lan.connect_active_notify(move |row| {
                manager.set_nearby_lan_enabled(row.is_active());
            });
        }
        group.add(&lan);
    }

    group
}

fn trusted_build_group() -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder()
        .title("Test build")
        .description("For trusted testing only.")
        .build()
}

fn media_group(prefs: &PreferencesSnapshot, manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Media").build();

    let enabled = adw::SwitchRow::builder().title("Image proxy").build();
    enabled.set_active(prefs.image_proxy_enabled);
    {
        let manager = manager.clone();
        enabled.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetImageProxyEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&enabled);

    let url = adw::EntryRow::builder().title("Proxy URL").build();
    url.set_text(&prefs.image_proxy_url);
    let manager_for_apply = manager.clone();
    url.connect_apply(move |row| {
        manager_for_apply.dispatch(AppAction::SetImageProxyUrl {
            url: row.text().to_string(),
        });
    });
    group.add(&url);

    let key = adw::EntryRow::builder().title("Proxy key").build();
    let manager_for_key = manager.clone();
    key.connect_apply(move |row| {
        manager_for_key.dispatch(AppAction::SetImageProxyKeyHex {
            key_hex: row.text().to_string(),
        });
    });
    group.add(&key);

    let salt = adw::EntryRow::builder().title("Proxy salt").build();
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
        reset.connect_activated(move |_| {
            manager.dispatch(AppAction::ResetImageProxySettings);
        });
    }
    group.add(&reset);

    group
}

fn updates_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Updates").build();

    let version = adw::ActionRow::builder()
        .title("Current version")
        .subtitle(iris_chat_core::app_version())
        .subtitle_selectable(true)
        .build();
    group.add(&version);

    let check = adw::ActionRow::builder()
        .title("Check for updates")
        .subtitle("Compares the running version with the latest published release")
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name("software-update-available-symbolic");
    check.add_prefix(&icon);
    let status = gtk::Label::builder()
        .css_classes(["dim-label"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    check.add_suffix(&status);
    let busy = Rc::new(Cell::new(false));
    {
        let busy = busy.clone();
        let status = status.clone();
        check.connect_activated(move |_| {
            if busy.get() {
                return;
            }
            busy.set(true);
            status.set_text("Checking…");
            let busy = busy.clone();
            let status = status.clone();
            glib::MainContext::default().spawn_local(async move {
                let summary = run_update_check().await;
                status.set_text(&summary);
                busy.set(false);
            });
        });
    }
    group.add(&check);

    group
}

const IRIS_UPDATE_REFERENCE: &str =
    "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest";

async fn run_update_check() -> String {
    let current = iris_chat_core::app_version();
    if is_dev_placeholder_version(&current) {
        return "Up to date".to_string();
    }
    let result = gtk::gio::spawn_blocking(move || {
        std::process::Command::new("htree")
            .args([
                "install",
                IRIS_UPDATE_REFERENCE,
                "--check",
                "--current-version",
                &current,
            ])
            .output()
    })
    .await;
    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}\n{stderr}");
            if output.status.success() {
                if combined.contains("up to date") || combined.contains("Up to date") {
                    "Up to date".to_string()
                } else if let Some(line) = combined
                    .lines()
                    .find(|line| line.to_lowercase().contains("available"))
                {
                    line.trim().to_string()
                } else {
                    "Up to date".to_string()
                }
            } else {
                "Update check failed".to_string()
            }
        }
        Ok(Err(_)) => "htree not found — install hashtree-cli".to_string(),
        Err(_) => "Update check cancelled".to_string(),
    }
}

/// Releases are tagged "YYYY.M.D[.N]". Dev builds fall back to the crate's
/// own semver (currently 0.1.x), which would otherwise look "older" than
/// every release and surface a misleading update prompt.
fn is_dev_placeholder_version(value: &str) -> bool {
    let major = value
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    major < 2000
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
    account: &iris_chat_core::AccountSnapshot,
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Profile").build();

    let avatar_row = adw::ActionRow::new();
    let avatar = adw::Avatar::new(56, Some(&account.display_name), true);
    if let Some(url) = account.picture_url.as_ref() {
        let proxied = iris_chat_core::proxied_image_url(
            url.clone(),
            prefs.clone(),
            Some(112),
            Some(112),
            true,
        );
        image_cache::fetch_into_avatar(&avatar, &proxied);
    }
    avatar_row.add_prefix(&avatar);
    avatar_row.set_title(if account.display_name.is_empty() {
        "Profile"
    } else {
        &account.display_name
    });

    let change_pic = gtk::Button::with_label("Change photo");
    change_pic.add_css_class("flat");
    change_pic.set_valign(gtk::Align::Center);
    let manager_for_pic = manager.clone();
    change_pic.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder()
            .title("Choose profile picture")
            .build();
        let manager = manager_for_pic.clone();
        dialog.open(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let Ok(file) = result else { return };
                if let Some(path) = file.path() {
                    manager.dispatch(AppAction::UploadProfilePicture {
                        file_path: path.to_string_lossy().to_string(),
                    });
                }
            },
        );
    });
    avatar_row.add_suffix(&change_pic);
    group.add(&avatar_row);

    let name_row = adw::EntryRow::builder().title("Display name").build();
    name_row.set_text(&account.display_name);
    let about_row = adw::PreferencesRow::new();
    let about_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    about_box.set_margin_top(10);
    about_box.set_margin_bottom(10);
    about_box.set_margin_start(12);
    about_box.set_margin_end(12);
    let about_label = gtk::Label::new(Some("About"));
    about_label.add_css_class("caption-heading");
    about_label.set_halign(gtk::Align::Start);
    about_box.append(&about_label);
    let about_view = gtk::TextView::new();
    about_view.set_wrap_mode(gtk::WrapMode::WordChar);
    about_view.set_size_request(-1, 88);
    about_view
        .buffer()
        .set_text(account.about.as_deref().unwrap_or(""));
    about_box.append(&about_view);
    about_row.set_child(Some(&about_box));
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_valign(gtk::Align::Center);
    let manager_for_save = manager.clone();
    let row_for_save = name_row.clone();
    let about_buffer_for_save = about_view.buffer();
    let picture_url = account.picture_url.clone();
    save.connect_clicked(move |_| {
        let value = row_for_save.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        manager_for_save.dispatch(AppAction::UpdateProfileMetadata {
            name: value,
            picture_url: picture_url.clone(),
            about: normalize_text_buffer(&about_buffer_for_save),
        });
    });
    name_row.add_suffix(&save);

    let manager_for_apply = manager.clone();
    let picture_url = account.picture_url.clone();
    let about_buffer_for_apply = about_view.buffer();
    name_row.connect_apply(move |row| {
        let value = row.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        manager_for_apply.dispatch(AppAction::UpdateProfileMetadata {
            name: value,
            picture_url: picture_url.clone(),
            about: normalize_text_buffer(&about_buffer_for_apply),
        });
    });
    group.add(&name_row);
    group.add(&about_row);

    let qr_row = adw::ActionRow::builder()
        .title("Show QR code")
        .activatable(true)
        .build();
    let qr_icon = gtk::Image::from_icon_name("preferences-other-symbolic");
    qr_row.add_prefix(&qr_icon);
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("dim-label");
    qr_row.add_suffix(&chevron);
    let profile_url = iris_chat_profile_url(&account.npub);
    let display_name = account.display_name.clone();
    let manager_for_qr = manager.clone();
    qr_row.connect_activated(move |row| {
        let parent = row.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        present_qr_dialog(
            parent.as_ref(),
            &display_name,
            &profile_url,
            manager_for_qr.clone(),
        );
    });
    group.add(&qr_row);

    let copy_user_id_row = adw::ActionRow::builder()
        .title("Copy user ID")
        .activatable(true)
        .build();
    let copy_icon = gtk::Image::from_icon_name("edit-copy-symbolic");
    copy_user_id_row.add_prefix(&copy_icon);
    let user_id = account.npub.clone();
    copy_user_id_row.connect_activated(move |_| clipboard::copy(&user_id));
    group.add(&copy_user_id_row);

    group
}

fn normalize_text_buffer(buffer: &gtk::TextBuffer) -> Option<String> {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    let text = buffer.text(&start, &end, true);
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn present_qr_dialog(
    parent: Option<&gtk::Window>,
    name: &str,
    profile_url: &str,
    manager: Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder().content_width(360).build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_halign(gtk::Align::Center);

    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tabs.set_halign(gtk::Align::Center);
    let code_tab = gtk::Button::with_label("Code");
    code_tab.add_css_class("pill");
    code_tab.set_width_request(104);
    tabs.append(&code_tab);

    let dialog_for_scan = dialog.clone();
    let manager_for_scan = manager.clone();
    let scan_tab = scan_qr_button("Scan", move |text| {
        manager_for_scan.dispatch(chat_input_action(text.trim()));
        dialog_for_scan.close();
    });
    scan_tab.set_width_request(104);
    tabs.append(&scan_tab);
    content.append(&tabs);

    let badge = gtk::Box::new(gtk::Orientation::Vertical, 8);
    badge.add_css_class("card");
    badge.set_width_request(296);
    badge.set_halign(gtk::Align::Center);
    badge.set_margin_top(8);
    badge.set_margin_bottom(4);
    badge.set_margin_start(0);
    badge.set_margin_end(0);

    let qr_widget = qr::build(profile_url, 216);
    qr_widget.set_margin_top(32);
    qr_widget.set_margin_start(40);
    qr_widget.set_margin_end(40);
    badge.append(&qr_widget);

    let name_button = gtk::Button::new();
    name_button.add_css_class("flat");
    name_button.set_halign(gtk::Align::Center);
    name_button.set_margin_top(8);
    name_button.set_margin_bottom(20);
    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    name_row.set_halign(gtk::Align::Center);
    let copy_icon = gtk::Image::from_icon_name("edit-copy-symbolic");
    name_row.append(&copy_icon);
    let name_label = gtk::Label::new(Some(if name.is_empty() { "User ID" } else { name }));
    name_label.add_css_class("title-3");
    name_row.append(&name_label);
    name_button.set_child(Some(&name_row));
    let profile_url_for_name_copy = profile_url.to_string();
    name_button.connect_clicked(move |_| clipboard::copy(&profile_url_for_name_copy));
    badge.append(&name_button);
    content.append(&badge);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.set_halign(gtk::Align::Center);
    let copy = gtk::Button::with_label("Copy");
    copy.add_css_class("pill");
    let profile_url_owned = profile_url.to_string();
    copy.connect_clicked(move |_| clipboard::copy(&profile_url_owned));
    actions.append(&copy);
    let dialog_for_action_scan = dialog.clone();
    let manager_for_action_scan = manager.clone();
    let scan = scan_qr_button("Scan", move |text| {
        manager_for_action_scan.dispatch(chat_input_action(text.trim()));
        dialog_for_action_scan.close();
    });
    scan.add_css_class("suggested-action");
    actions.append(&scan);
    content.append(&actions);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn messaging_group(prefs: &PreferencesSnapshot, manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Messaging").build();

    let accept_requests = adw::SwitchRow::builder()
        .title("Accept message requests from unknowns")
        .build();
    accept_requests.set_active(prefs.accept_unknown_direct_messages);
    {
        let manager = manager.clone();
        accept_requests.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetAcceptUnknownDirectMessages {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&accept_requests);

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

    if startup::is_supported() {
        let startup_row = adw::SwitchRow::builder().title("Open at login").build();
        startup_row.set_active(prefs.startup_at_login_enabled);
        {
            let manager = manager.clone();
            let reverting = Rc::new(Cell::new(false));
            startup_row.connect_active_notify(move |row| {
                if reverting.replace(false) {
                    return;
                }
                let enabled = row.is_active();
                if startup::set_enabled(enabled).is_ok() {
                    manager.dispatch(AppAction::SetStartupAtLoginEnabled { enabled });
                } else {
                    reverting.set(true);
                    row.set_active(!enabled);
                }
            });
        }
        group.add(&startup_row);
    }

    group
}

fn notifications_group(
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Notifications")
        .build();

    let notifications = adw::SwitchRow::builder().title("Enabled").build();
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
    let group = adw::PreferencesGroup::builder()
        .title("Message servers")
        .build();

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

    let add_row = adw::EntryRow::builder().title("Add server").build();
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

fn account_data_group(manager: &Rc<AppManager>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Account data")
        .build();

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
        logout.connect_activated(move |row| {
            let parent = row
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            confirm_delete_app_data(parent.as_ref(), &manager);
        });
    }
    group.add(&logout);

    group
}

fn about_group(state: &AppState) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("About").build();

    let source = adw::ActionRow::builder()
        .title("Source code")
        .subtitle("Iris Chat source code")
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name("code-context-symbolic");
    source.add_prefix(&icon);
    source.connect_activated(|row| {
        let parent = row
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        gtk::UriLauncher::new(IRIS_SOURCE_URL).launch(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            |result| {
                if let Err(err) = result {
                    eprintln!("Could not open source link: {err}");
                }
            },
        );
    });
    group.add(&source);

    let version = adw::ActionRow::builder()
        .title("Version")
        .subtitle(iris_chat_core::app_version())
        .subtitle_selectable(true)
        .build();
    group.add(&version);

    if let Some(net) = state.network_status.as_ref() {
        let status = adw::ActionRow::builder()
            .title("Network")
            .subtitle(format!(
                "{} · {} servers · {} updates",
                if net.syncing { "syncing" } else { "idle" },
                net.relay_urls.len(),
                net.recent_event_count
            ))
            .build();
        group.add(&status);
    }

    group
}
