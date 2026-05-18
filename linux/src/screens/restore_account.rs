use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Restore profile"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let hint = gtk::Label::new(Some("Paste your secret key."));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    container.append(&hint);

    let nsec = entry("Secret key");
    nsec.set_visibility(false);
    nsec.set_input_purpose(gtk::InputPurpose::Password);
    container.append(&nsec);

    let secret_key_info = gtk::Label::new(Some("Secret key = nostr nsec"));
    secret_key_info.add_css_class("dim-label");
    secret_key_info.set_halign(gtk::Align::Start);
    container.append(&secret_key_info);

    let busy = state.busy.restoring_session;
    let submit = primary_button(if busy {
        "Restoring…"
    } else {
        "Restore profile"
    });
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
