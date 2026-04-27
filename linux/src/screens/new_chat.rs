use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Start a chat"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let hint = gtk::Label::new(Some("Paste an npub or invite link."));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    container.append(&hint);

    let peer = entry("npub or invite link");
    container.append(&peer);

    let busy = state.busy.creating_chat || state.busy.accepting_invite;
    let submit = primary_button(if busy { "Opening…" } else { "Open chat" });
    submit.set_sensitive(!busy);

    let manager_for_submit = manager.clone();
    let peer_for_submit = peer.clone();
    submit.connect_clicked(move |btn| {
        let value = peer_for_submit.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        manager_for_submit.dispatch(action_for(value));
    });

    let manager_for_enter = manager.clone();
    let submit_for_enter = submit.clone();
    peer.connect_activate(move |entry| {
        let value = entry.text().trim().to_string();
        if value.is_empty() || !submit_for_enter.is_sensitive() {
            return;
        }
        submit_for_enter.set_sensitive(false);
        manager_for_enter.dispatch(action_for(value));
    });

    container.append(&submit);
    container.upcast()
}

fn action_for(input: String) -> AppAction {
    let lower = input.to_lowercase();
    if lower.contains("://") && lower.contains("#") {
        AppAction::AcceptInvite { invite_input: input }
    } else {
        AppAction::CreateChat { peer_input: input }
    }
}
