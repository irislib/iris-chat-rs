use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, scan_qr_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Add this device"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let hint = gtk::Label::new(Some(
        "Paste your owner npub from another signed-in device. The owner will be asked to approve this device.",
    ));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    container.append(&hint);

    let owner = entry("Owner npub");
    container.append(&owner);

    let owner_for_scan = owner.clone();
    let scan = scan_qr_button("Scan QR from image", move |text| {
        owner_for_scan.set_text(&text);
    });
    container.append(&scan);

    let busy = state.busy.linking_device;
    let submit = primary_button(if busy { "Linking…" } else { "Link this device" });
    submit.set_sensitive(!busy);

    let manager_for_submit = manager.clone();
    let owner_for_submit = owner.clone();
    submit.connect_clicked(move |btn| {
        let value = owner_for_submit.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        manager_for_submit.dispatch(AppAction::StartLinkedDevice { owner_input: value });
    });

    let manager_for_enter = manager.clone();
    let submit_for_enter = submit.clone();
    owner.connect_activate(move |entry| {
        let value = entry.text().trim().to_string();
        if value.is_empty() || !submit_for_enter.is_sensitive() {
            return;
        }
        submit_for_enter.set_sensitive(false);
        manager_for_enter.dispatch(AppAction::StartLinkedDevice { owner_input: value });
    });

    container.append(&submit);

    container.upcast()
}
