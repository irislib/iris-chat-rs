use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::platform::clipboard;
use crate::screens::{entry, pill_button, primary_button, scan_qr_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Join chat"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let invite = entry("Invite link");
    container.append(&invite);

    let paste = pill_button("Paste");
    let invite_for_paste = invite.clone();
    paste.connect_clicked(move |_| {
        let entry = invite_for_paste.clone();
        clipboard::paste(move |value| {
            if !value.is_empty() {
                entry.set_text(&value);
            }
        });
    });
    container.append(&paste);

    let invite_for_scan = invite.clone();
    let scan = scan_qr_button("Scan QR", move |text| {
        invite_for_scan.set_text(&text);
    });
    container.append(&scan);

    let busy = state.busy.accepting_invite;
    let submit = primary_button(if busy { "Joining…" } else { "Join chat" });
    submit.set_sensitive(!busy);

    let manager_for_submit = manager.clone();
    let invite_for_submit = invite.clone();
    submit.connect_clicked(move |btn| {
        let value = invite_for_submit.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        manager_for_submit.dispatch(AppAction::AcceptInvite {
            invite_input: value,
        });
    });

    let manager_for_enter = manager.clone();
    let submit_for_enter = submit.clone();
    invite.connect_activate(move |entry| {
        let value = entry.text().trim().to_string();
        if value.is_empty() || !submit_for_enter.is_sensitive() {
            return;
        }
        submit_for_enter.set_sensitive(false);
        manager_for_enter.dispatch(AppAction::AcceptInvite {
            invite_input: value,
        });
    });

    container.append(&submit);
    container.upcast()
}
