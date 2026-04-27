use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Restore account"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let hint = gtk::Label::new(Some("Use your owner secret key to recover your account on this device."));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    container.append(&hint);

    let nsec = entry("Owner nsec");
    container.append(&nsec);

    let busy = state.busy.restoring_session;
    let submit = primary_button(if busy { "Restoring…" } else { "Restore account" });
    submit.set_sensitive(!busy);

    let manager_for_submit = manager.clone();
    let nsec_for_submit = nsec.clone();
    submit.connect_clicked(move |btn| {
        let value = nsec_for_submit.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        manager_for_submit.dispatch(AppAction::RestoreSession { owner_nsec: value });
    });

    let manager_for_enter = manager.clone();
    let submit_for_enter = submit.clone();
    nsec.connect_activate(move |entry| {
        let value = entry.text().trim().to_string();
        if value.is_empty() || !submit_for_enter.is_sensitive() {
            return;
        }
        submit_for_enter.set_sensitive(false);
        manager_for_enter.dispatch(AppAction::RestoreSession { owner_nsec: value });
    });

    container.append(&submit);

    container.upcast()
}
