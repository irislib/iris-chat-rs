use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use iris_chat_core::{peer_input_to_npub, AppAction, CurrentChatSnapshot, PreferencesSnapshot};

use crate::app_manager::AppManager;

const IRIS_SUPPORT_EMAIL: &str = "irismessenger@pm.me";

pub(super) fn is_user_blocked(preferences: &PreferencesSnapshot, chat_id: &str) -> bool {
    let normalized = chat_id.trim().to_lowercase();
    !normalized.is_empty()
        && preferences
            .blocked_owner_pubkeys
            .iter()
            .any(|blocked| blocked.eq_ignore_ascii_case(&normalized))
}

pub(super) fn blocked_bar(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.add_css_class("card");
    outer.set_margin_top(8);
    outer.set_margin_bottom(8);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_hexpand(true);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(10);
    row.set_margin_start(12);
    row.set_margin_end(12);
    let icon = gtk::Image::from_icon_name("action-unavailable-symbolic");
    icon.add_css_class("error");
    row.append(&icon);
    let label = gtk::Label::new(Some("User blocked"));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    outer.append(&row);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_start(12);
    actions.set_margin_end(12);
    actions.set_margin_bottom(10);

    let delete = gtk::Button::with_label("Delete chat");
    delete.add_css_class("destructive-action");
    delete.set_hexpand(true);
    let manager_for_delete = manager.clone();
    let chat_id_for_delete = chat.chat_id.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteChat {
            chat_id: chat_id_for_delete.clone(),
        });
        manager_for_delete.dispatch(AppAction::NavigateBack);
    });
    actions.append(&delete);

    let unblock = gtk::Button::with_label("Unblock");
    unblock.add_css_class("suggested-action");
    unblock.set_hexpand(true);
    let manager_for_unblock = manager.clone();
    let chat_id_for_unblock = chat.chat_id.clone();
    unblock.connect_clicked(move |_| {
        manager_for_unblock.dispatch(AppAction::SetUserBlocked {
            owner_pubkey_hex: chat_id_for_unblock.clone(),
            blocked: false,
        });
    });
    actions.append(&unblock);

    outer.append(&actions);
    outer.upcast()
}

pub(super) fn message_request_bar(
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.add_css_class("card");
    outer.set_margin_top(8);
    outer.set_margin_bottom(8);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_hexpand(true);

    let label = gtk::Label::new(Some(&format!("Message request from {}", chat.display_name)));
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_xalign(0.5);
    label.set_margin_top(10);
    label.set_margin_start(12);
    label.set_margin_end(12);
    outer.append(&label);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_start(12);
    actions.set_margin_end(12);
    actions.set_margin_bottom(10);

    let block = gtk::Button::with_label("Block");
    block.add_css_class("destructive-action");
    block.set_hexpand(true);
    let manager_for_block = manager.clone();
    let chat_id_for_block = chat.chat_id.clone();
    let display_name_for_block = chat.display_name.clone();
    block.connect_clicked(move |btn| {
        let parent = btn
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_block_user_dialog(
            parent.as_ref(),
            chat_id_for_block.clone(),
            display_name_for_block.clone(),
            manager_for_block.clone(),
        );
    });
    actions.append(&block);

    let report = gtk::Button::with_label("Block and report");
    report.add_css_class("destructive-action");
    report.set_hexpand(true);
    let manager_for_report = manager.clone();
    let chat_id_for_report = chat.chat_id.clone();
    let display_name_for_report = chat.display_name.clone();
    report.connect_clicked(move |btn| {
        let parent = btn
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_block_and_report_user_dialog(
            parent.as_ref(),
            chat_id_for_report.clone(),
            display_name_for_report.clone(),
            manager_for_report.clone(),
        );
    });
    actions.append(&report);

    let accept = gtk::Button::with_label("Accept");
    accept.add_css_class("suggested-action");
    accept.set_hexpand(true);
    let manager_for_accept = manager.clone();
    let chat_id_for_accept = chat.chat_id.clone();
    accept.connect_clicked(move |_| {
        manager_for_accept.dispatch(AppAction::SetMessageRequestAccepted {
            chat_id: chat_id_for_accept.clone(),
        });
    });
    actions.append(&accept);

    outer.append(&actions);
    outer.upcast()
}

pub(super) fn present_block_user_dialog(
    parent: Option<&gtk::Window>,
    chat_id: String,
    display_name: String,
    manager: Rc<AppManager>,
) {
    let title = format!("Block {display_name}?");
    let dialog = adw::Dialog::builder()
        .title(&title)
        .content_width(340)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let message = gtk::Label::new(Some("They will not be able to message you."));
    message.add_css_class("dim-label");
    message.set_wrap(true);
    message.set_xalign(0.0);
    content.append(&message);

    let block = dialog_action_button("Block", true);
    let manager_for_block = manager.clone();
    let chat_id_for_block = chat_id.clone();
    let dialog_for_block = dialog.clone();
    block.connect_clicked(move |_| {
        manager_for_block.dispatch(AppAction::SetUserBlocked {
            owner_pubkey_hex: chat_id_for_block.clone(),
            blocked: true,
        });
        dialog_for_block.close();
    });
    content.append(&block);

    let report = dialog_action_button("Block and report", true);
    let manager_for_report = manager.clone();
    let chat_id_for_report = chat_id.clone();
    let display_name_for_report = display_name.clone();
    let dialog_for_report = dialog.clone();
    report.connect_clicked(move |_| {
        report_user(
            &chat_id_for_report,
            &display_name_for_report,
            true,
            &manager_for_report,
        );
        dialog_for_report.close();
    });
    content.append(&report);

    let delete = dialog_action_button("Delete chat", true);
    let manager_for_delete = manager.clone();
    let chat_id_for_delete = chat_id.clone();
    let dialog_for_delete = dialog.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteChat {
            chat_id: chat_id_for_delete.clone(),
        });
        manager_for_delete.dispatch(AppAction::NavigateBack);
        dialog_for_delete.close();
    });
    content.append(&delete);

    let cancel = dialog_action_button("Cancel", false);
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });
    content.append(&cancel);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

pub(super) fn present_report_user_dialog(
    parent: Option<&gtk::Window>,
    chat_id: String,
    display_name: String,
    manager: Rc<AppManager>,
) {
    let title = format!("Report {display_name}?");
    let dialog = adw::Dialog::builder()
        .title(&title)
        .content_width(340)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let message = gtk::Label::new(Some("This prepares a report for support."));
    message.add_css_class("dim-label");
    message.set_wrap(true);
    message.set_xalign(0.0);
    content.append(&message);

    let report = dialog_action_button("Report", true);
    let manager_for_report = manager.clone();
    let chat_id_for_report = chat_id.clone();
    let display_name_for_report = display_name.clone();
    let dialog_for_report = dialog.clone();
    report.connect_clicked(move |_| {
        report_user(
            &chat_id_for_report,
            &display_name_for_report,
            false,
            &manager_for_report,
        );
        dialog_for_report.close();
    });
    content.append(&report);

    let report_block = dialog_action_button("Block and report", true);
    let manager_for_report_block = manager.clone();
    let chat_id_for_report_block = chat_id.clone();
    let display_name_for_report_block = display_name.clone();
    let dialog_for_report_block = dialog.clone();
    report_block.connect_clicked(move |_| {
        report_user(
            &chat_id_for_report_block,
            &display_name_for_report_block,
            true,
            &manager_for_report_block,
        );
        dialog_for_report_block.close();
    });
    content.append(&report_block);

    let delete = dialog_action_button("Delete chat", true);
    let manager_for_delete = manager.clone();
    let chat_id_for_delete = chat_id.clone();
    let dialog_for_delete = dialog.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteChat {
            chat_id: chat_id_for_delete.clone(),
        });
        manager_for_delete.dispatch(AppAction::NavigateBack);
        dialog_for_delete.close();
    });
    content.append(&delete);

    let cancel = dialog_action_button("Cancel", false);
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });
    content.append(&cancel);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn present_block_and_report_user_dialog(
    parent: Option<&gtk::Window>,
    chat_id: String,
    display_name: String,
    manager: Rc<AppManager>,
) {
    let title = format!("Block and report {display_name}?");
    let dialog = adw::Dialog::builder()
        .title(&title)
        .content_width(340)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let message = gtk::Label::new(Some(
        "This prepares a report for support and blocks this user.",
    ));
    message.add_css_class("dim-label");
    message.set_wrap(true);
    message.set_xalign(0.0);
    content.append(&message);

    let report_block = dialog_action_button("Block and report", true);
    let manager_for_report_block = manager.clone();
    let chat_id_for_report_block = chat_id.clone();
    let display_name_for_report_block = display_name.clone();
    let dialog_for_report_block = dialog.clone();
    report_block.connect_clicked(move |_| {
        report_user(
            &chat_id_for_report_block,
            &display_name_for_report_block,
            true,
            &manager_for_report_block,
        );
        dialog_for_report_block.close();
    });
    content.append(&report_block);

    let delete = dialog_action_button("Delete chat", true);
    let manager_for_delete = manager.clone();
    let chat_id_for_delete = chat_id.clone();
    let dialog_for_delete = dialog.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteChat {
            chat_id: chat_id_for_delete.clone(),
        });
        manager_for_delete.dispatch(AppAction::NavigateBack);
        dialog_for_delete.close();
    });
    content.append(&delete);

    let cancel = dialog_action_button("Cancel", false);
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });
    content.append(&cancel);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn dialog_action_button(label: &str, destructive: bool) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_halign(gtk::Align::Fill);
    if destructive {
        button.add_css_class("destructive-action");
    }
    button
}

fn report_user(chat_id: &str, display_name: &str, block: bool, manager: &Rc<AppManager>) {
    if block {
        manager.dispatch(AppAction::SetUserBlocked {
            owner_pubkey_hex: chat_id.to_string(),
            blocked: true,
        });
    }

    let user_id = peer_input_to_npub(chat_id.to_string());
    let body = format!(
        "Reported user: {display_name}\nUser ID: {user_id}\nApp: Iris Chat Linux\n\nWhat happened:\n"
    );
    let uri = format!(
        "mailto:{IRIS_SUPPORT_EMAIL}?subject={}&body={}",
        percent_encode("Iris Chat user report"),
        percent_encode(&body),
    );
    if gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE).is_err() {
        crate::platform::clipboard::copy(&format!(
            "To: {IRIS_SUPPORT_EMAIL}\nSubject: Iris Chat user report\n\n{body}"
        ));
    }
}

fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}
