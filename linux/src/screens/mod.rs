use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppState, Screen};

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
        Screen::CreateAccount => "Create account",
        Screen::RestoreAccount => "Restore account",
        Screen::AddDevice => "Add device",
        Screen::ChatList => "Chats",
        Screen::NewChat => "New chat",
        Screen::NewGroup => "New group",
        Screen::CreateInvite => "Invite link",
        Screen::JoinInvite => "Join invite",
        Screen::Settings => "Settings",
        Screen::Chat { .. } => "Chat",
        Screen::GroupDetails { .. } => "Group",
        Screen::DeviceRoster => "Devices",
        Screen::AwaitingDeviceApproval => "Awaiting approval",
        Screen::DeviceRevoked => "Device revoked",
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

pub(crate) fn dispatch_on_click<F>(button: &gtk::Button, manager: &Rc<AppManager>, action: F)
where
    F: Fn() -> ndr_demo_core::AppAction + 'static,
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
        crate::platform::qr_scan::pick_and_decode(parent.as_ref(), move |text| {
            (on_result)(text);
        });
    });
    btn
}
