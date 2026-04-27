use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppState, Screen};

use crate::app_manager::AppManager;

mod add_device;
mod chat;
mod chat_list;
mod create_account;
mod new_chat;
mod restore_account;
mod welcome;

pub fn render(screen: &Screen, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    match screen {
        Screen::Welcome => welcome::render(manager),
        Screen::CreateAccount => create_account::render(state, manager),
        Screen::RestoreAccount => restore_account::render(state, manager),
        Screen::AddDevice => add_device::render(state, manager),
        Screen::ChatList => chat_list::render(state, manager),
        Screen::NewChat => new_chat::render(state, manager),
        Screen::Chat { chat_id } => chat::render(chat_id, state, manager),
        other => placeholder(other),
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

fn placeholder(screen: &Screen) -> gtk::Widget {
    let label = gtk::Label::new(Some(&format!("{}\n(not implemented yet)", title(screen))));
    label.set_vexpand(true);
    label.set_valign(gtk::Align::Center);
    label.set_halign(gtk::Align::Center);
    label.add_css_class("dim-label");
    label.upcast()
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
