use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Create account"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let name = entry("Display name");
    container.append(&name);

    let busy = state.busy.creating_account;
    let submit = primary_button(if busy { "Creating…" } else { "Create account" });
    submit.set_sensitive(!busy);

    let manager_for_submit = manager.clone();
    let name_for_submit = name.clone();
    submit.connect_clicked(move |btn| {
        let value = name_for_submit.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        manager_for_submit.dispatch(AppAction::CreateAccount { name: value });
    });

    let manager_for_enter = manager.clone();
    let submit_for_enter = submit.clone();
    name.connect_activate(move |entry| {
        let value = entry.text().trim().to_string();
        if value.is_empty() || !submit_for_enter.is_sensitive() {
            return;
        }
        submit_for_enter.set_sensitive(false);
        manager_for_enter.dispatch(AppAction::CreateAccount { name: value });
    });

    container.append(&submit);

    container.upcast()
}
