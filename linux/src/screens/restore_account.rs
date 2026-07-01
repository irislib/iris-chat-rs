use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::screens::{entry, screen_container};

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

    let busy = state.busy.restoring_session;
    let nsec = entry("Secret key");
    nsec.set_visibility(false);
    nsec.set_input_purpose(gtk::InputPurpose::Password);
    nsec.set_sensitive(!busy);
    container.append(&nsec);

    let secret_key_info = gtk::Label::new(Some("Secret key = nostr nsec"));
    secret_key_info.add_css_class("dim-label");
    secret_key_info.set_halign(gtk::Align::Start);
    container.append(&secret_key_info);

    let last_submitted_secret: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let manager_for_change = manager.clone();
    nsec.connect_changed(move |entry| {
        if busy {
            return;
        }
        let current = entry.text().trim().to_string();
        if !should_auto_submit_secret(&current) {
            return;
        }
        if last_submitted_secret.borrow().as_deref() == Some(current.as_str()) {
            return;
        }
        *last_submitted_secret.borrow_mut() = Some(current.clone());
        entry.set_sensitive(false);
        manager_for_change.dispatch(AppAction::RestoreSession {
            owner_nsec: current,
        });
    });

    container.upcast()
}

fn should_auto_submit_secret(current: &str) -> bool {
    if current.is_empty() {
        return false;
    }
    let lower = current.to_ascii_lowercase();
    if lower.starts_with("nsec1") {
        return current.len() >= 63;
    }
    current.len() == 64 && current.chars().all(|ch| ch.is_ascii_hexdigit())
}
