use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState, Screen};

use crate::app_manager::AppManager;

mod add_device;
pub mod chat;
pub(crate) mod chat_list;
mod create_account;
mod create_invite;
mod device_revoked;
mod device_roster;
mod group_details;
mod join_invite;
mod nearby;
mod new_chat;
mod new_group;
mod remote_signer;
mod restore_account;
pub(crate) mod settings;
mod sync_status;
mod welcome;

pub fn render(screen: &Screen, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    match screen {
        Screen::Welcome => welcome::render(manager),
        Screen::CreateAccount => create_account::render(state, manager),
        Screen::RestoreAccount => restore_account::render(state, manager),
        Screen::RemoteSigner => remote_signer::render(state, manager),
        Screen::AddDevice => add_device::render(state, manager),
        Screen::ChatList => chat_list::render(state, manager),
        Screen::NewChat => new_chat::render(state, manager),
        Screen::NewGroup => new_group::render(state, manager),
        Screen::CreateInvite => create_invite::render(state, manager),
        Screen::JoinInvite => join_invite::render(state, manager),
        Screen::Chat { chat_id } => chat::render(chat_id, state, manager),
        Screen::DirectChatInfo { chat_id } => chat::render(chat_id, state, manager),
        Screen::GroupDetails { group_id } => group_details::render(group_id, state, manager),
        Screen::DeviceRoster => device_roster::render(state, manager),
        Screen::DeviceRevoked => device_revoked::render(state, manager),
        Screen::Settings => settings::render(state, manager),
    }
}

pub fn title(screen: &Screen) -> &'static str {
    match screen {
        Screen::Welcome => "Welcome",
        Screen::CreateAccount => "Create profile",
        Screen::RestoreAccount => "Restore profile",
        Screen::RemoteSigner => "Signer app/device",
        Screen::AddDevice => "Link device",
        Screen::ChatList => "Chats",
        Screen::NewChat => "New chat",
        Screen::NewGroup => "New group",
        Screen::CreateInvite => "Invite",
        Screen::JoinInvite => "Join invite",
        Screen::Settings => "Settings",
        Screen::Chat { .. } => "Chat",
        Screen::DirectChatInfo { .. } => "Details",
        Screen::GroupDetails { .. } => "Group",
        Screen::DeviceRoster => "Devices",
        Screen::DeviceRevoked => "Device removed",
    }
}

pub(crate) fn screen_container() -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
    container.set_margin_top(24);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);
    container.set_valign(gtk::Align::Start);
    container.set_hexpand(true);
    container
}

pub(crate) fn pill_button(label: &str) -> gtk::Button {
    let btn = gtk::Button::with_label(label);
    btn.add_css_class("pill");
    btn.set_height_request(44);
    btn
}

pub(crate) fn primary_button(label: &str) -> gtk::Button {
    let btn = pill_button(label);
    btn.add_css_class("suggested-action");
    btn
}

pub(crate) fn entry(placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_height_request(40);
    entry
}

/// Convert free-text input from any chat-action field (New Chat, the
/// search bar's shortcut row, deep-link handler) into the right
/// `AppAction`. The core does the parsing — we just adapt its enum
/// into a one-shot dispatch so callers can't accidentally diverge on
/// how an invite URL vs an npub is recognized.
pub(crate) fn chat_input_action(input: &str) -> iris_chat_core::AppAction {
    use iris_chat_core::ChatInputShortcut;
    match iris_chat_core::classify_chat_input(input.to_string()) {
        Some(ChatInputShortcut::Invite { invite_input, .. }) => {
            iris_chat_core::AppAction::AcceptInvite { invite_input }
        }
        Some(ChatInputShortcut::DirectPeer { peer_input, .. }) => {
            iris_chat_core::AppAction::CreateChat { peer_input }
        }
        // Unparseable text — let the core surface its own validation
        // error via the existing CreateChat path. Matches the legacy
        // behaviour for callers that hand-typed unrecognized text.
        None => iris_chat_core::AppAction::CreateChat {
            peer_input: input.to_string(),
        },
    }
}

pub(crate) fn confirm_delete_app_data(parent: Option<&gtk::Window>, manager: &Rc<AppManager>) {
    let dialog = adw::Dialog::builder()
        .title("Log out and delete local data?")
        .content_width(340)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let title = gtk::Label::new(Some("Log out and delete local data?"));
    title.add_css_class("title-2");
    title.set_halign(gtk::Align::Start);
    content.append(&title);

    let message = gtk::Label::new(Some(
        "This removes your secret keys, messages, and cached files from this device. Your other devices keep their data.",
    ));
    message.set_wrap(true);
    message.set_xalign(0.0);
    message.add_css_class("dim-label");
    content.append(&message);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);

    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("pill");
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    buttons.append(&cancel);

    let delete = gtk::Button::with_label("Log out");
    delete.add_css_class("pill");
    delete.add_css_class("destructive-action");
    {
        let manager = manager.clone();
        let dialog = dialog.clone();
        delete.connect_clicked(move |_| {
            manager.dispatch(AppAction::Logout);
            dialog.close();
        });
    }
    buttons.append(&delete);

    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.present(parent);
}

pub(crate) fn show_chat_mute_options(
    parent: Option<&gtk::Window>,
    manager: &Rc<AppManager>,
    chat_id: &str,
    muted: bool,
) {
    let dialog = adw::Dialog::builder()
        .title("Mute notifications")
        .content_width(300)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);
    let title = gtk::Label::new(Some("Mute notifications"));
    title.add_css_class("title-2");
    content.append(&title);
    if muted {
        let state = manager.current_state();
        let deadline = state
            .preferences
            .timed_chat_mutes
            .iter()
            .find(|mute| mute.chat_id == chat_id);
        let text = deadline
            .and_then(|mute| gtk::glib::DateTime::from_unix_local(mute.until_secs as i64).ok())
            .and_then(|date| date.format("%x %H:%M").ok())
            .map(|date| format!("Muted until {date}"))
            .unwrap_or_else(|| "Muted always".to_string());
        let status = gtk::Label::new(Some(&text));
        status.add_css_class("dim-label");
        content.append(&status);
    }
    let mut options = vec![
        ("1 hour", Some(3_600)),
        ("8 hours", Some(28_800)),
        ("1 day", Some(86_400)),
        ("1 week", Some(604_800)),
        ("Always", None),
    ];
    if muted {
        options.insert(0, ("Unmute", Some(0)));
    }
    for (label, seconds) in options {
        let button = gtk::Button::with_label(label);
        button.add_css_class("flat");
        let manager = manager.clone();
        let chat_id = chat_id.to_string();
        let dialog = dialog.clone();
        button.connect_clicked(move |_| {
            let action = match seconds {
                None => AppAction::SetChatMuted {
                    chat_id: chat_id.clone(),
                    muted: true,
                },
                Some(0) => AppAction::SetChatMuted {
                    chat_id: chat_id.clone(),
                    muted: false,
                },
                Some(duration) => AppAction::SetChatMuteUntil {
                    chat_id: chat_id.clone(),
                    until_secs: chat_list::unix_now().saturating_add(duration),
                },
            };
            manager.dispatch(action);
            dialog.close();
        });
        content.append(&button);
    }
    let cancel = gtk::Button::with_label("Cancel");
    let closing = dialog.clone();
    cancel.connect_clicked(move |_| {
        closing.close();
    });
    content.append(&cancel);
    dialog.set_child(Some(&content));
    crate::widgets::dialogs::present(&dialog, parent);
}

pub(crate) fn confirm_delete_chat(
    parent: Option<&gtk::Window>,
    manager: &Rc<AppManager>,
    chat_id: String,
) {
    let dialog = adw::Dialog::builder()
        .title("Delete chat?")
        .content_width(340)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let title = gtk::Label::new(Some("Delete chat?"));
    title.add_css_class("title-2");
    title.set_halign(gtk::Align::Start);
    content.append(&title);

    let message = gtk::Label::new(Some("This removes messages from this device."));
    message.set_wrap(true);
    message.set_xalign(0.0);
    message.add_css_class("dim-label");
    content.append(&message);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);

    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("pill");
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    buttons.append(&cancel);

    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("pill");
    delete.add_css_class("destructive-action");
    {
        let manager = manager.clone();
        let dialog = dialog.clone();
        delete.connect_clicked(move |_| {
            manager.dispatch(AppAction::DeleteChat {
                chat_id: chat_id.clone(),
            });
            dialog.close();
        });
    }
    buttons.append(&delete);

    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.present(parent);
}

pub(crate) fn dispatch_on_click<F>(button: &gtk::Button, manager: &Rc<AppManager>, action: F)
where
    F: Fn() -> iris_chat_core::AppAction + 'static,
{
    let manager = manager.clone();
    button.connect_clicked(move |_| {
        manager.dispatch(action());
    });
}

pub(crate) fn scan_qr_button<F: Fn(String) + 'static>(label: &str, on_result: F) -> gtk::Button {
    let btn = pill_button(label);
    let on_result = Rc::new(on_result);
    btn.connect_clicked(move |b| {
        let parent = b.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        let on_result = on_result.clone();
        crate::platform::qr_scan::open_scanner(parent.as_ref(), move |text| {
            (on_result)(text);
        });
    });
    btn
}

pub(crate) fn present_nearby(parent: Option<&gtk::Window>, manager: Rc<AppManager>) {
    nearby::present(parent, manager);
}

fn chat_details_actions(
    manager: &Rc<AppManager>,
    chat_id: &str,
    name: &str,
    close: impl Fn() + 'static,
) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    let search = adw::ActionRow::builder()
        .title("Search in chat")
        .activatable(true)
        .build();
    search.add_prefix(&gtk::Image::from_icon_name("system-search-symbolic"));
    let target = chat_id.to_string();
    let name = name.to_string();
    let searching = manager.clone();
    search.connect_activated(move |_| {
        searching.enter_chat_scope(target.clone(), name.clone());
        searching.dispatch(AppAction::UpdateScreenStack { stack: Vec::new() });
        close();
        searching.redraw_ui();
    });
    group.add(&search);
    if let Some(expiry) = chat::disappearing_messages_row(manager, chat_id) {
        group.add(&expiry);
    }
    let pin = adw::ActionRow::builder().activatable(true).build();
    let update_title = |row: &adw::ActionRow, pinned| {
        row.set_title(if pinned { "Unpin chat" } else { "Pin chat" })
    };
    update_title(
        &pin,
        manager
            .current_state()
            .preferences
            .pinned_chat_ids
            .contains(&chat_id.to_string()),
    );
    let target = chat_id.to_string();
    let manager = manager.clone();
    pin.connect_activated(move |row| {
        let pinned = !manager
            .current_state()
            .preferences
            .pinned_chat_ids
            .contains(&target);
        manager.dispatch(AppAction::SetChatPinned {
            chat_id: target.clone(),
            pinned,
        });
        update_title(row, pinned);
    });
    group.add(&pin);
    group.upcast()
}
