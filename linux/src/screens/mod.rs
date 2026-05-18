use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState, Screen};

use crate::app_manager::AppManager;

mod add_device;
mod awaiting_device_approval;
pub mod chat;
mod chat_list;
mod create_account;
mod create_invite;
mod device_revoked;
mod device_roster;
mod group_details;
mod join_invite;
mod nearby;
mod new_chat;
mod new_group;
mod restore_account;
mod settings;
mod welcome;

pub fn render(screen: &Screen, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    match screen {
        Screen::Welcome => welcome::render(manager),
        Screen::CreateAccount => create_account::render(state, manager),
        Screen::RestoreAccount => restore_account::render(state, manager),
        Screen::AddDevice => add_device::render(state, manager),
        Screen::ChatList => chat_list::render(state, manager),
        Screen::NewChat => new_chat::render(state, manager),
        Screen::NewGroup => new_group::render(state, manager),
        Screen::CreateInvite => create_invite::render(state, manager),
        Screen::JoinInvite => join_invite::render(state, manager),
        Screen::Chat { chat_id } => chat::render(chat_id, state, manager),
        Screen::GroupDetails { group_id } => group_details::render(group_id, state, manager),
        Screen::DeviceRoster => device_roster::render(state, manager),
        Screen::AwaitingDeviceApproval => awaiting_device_approval::render(state, manager),
        Screen::DeviceRevoked => device_revoked::render(state, manager),
        Screen::Settings => settings::render(state, manager),
    }
}

pub fn title(screen: &Screen) -> &'static str {
    match screen {
        Screen::Welcome => "Welcome",
        Screen::CreateAccount => "Create profile",
        Screen::RestoreAccount => "Restore profile",
        Screen::AddDevice => "Link device",
        Screen::ChatList => "Chats",
        Screen::NewChat => "New chat",
        Screen::NewGroup => "New group",
        Screen::CreateInvite => "Invite",
        Screen::JoinInvite => "Join invite",
        Screen::Settings => "Settings",
        Screen::Chat { .. } => "Chat",
        Screen::GroupDetails { .. } => "Group",
        Screen::DeviceRoster => "Devices",
        Screen::AwaitingDeviceApproval => "Awaiting approval",
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
        .title("Delete all local data?")
        .content_width(340)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let title = gtk::Label::new(Some("Delete all local data?"));
    title.add_css_class("title-2");
    title.set_halign(gtk::Align::Start);
    content.append(&title);

    let message = gtk::Label::new(Some(
        "This removes your secret keys, messages, and cached files from this device.",
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

    let delete = gtk::Button::with_label("Delete");
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
