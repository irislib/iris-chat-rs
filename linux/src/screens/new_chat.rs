use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState, Screen};

use crate::app_manager::AppManager;
use crate::platform::clipboard;
use crate::screens::{entry, pill_button, primary_button, scan_qr_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Start a chat"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let hint = gtk::Label::new(Some("Paste a user ID or invite link."));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    container.append(&hint);

    let peer = entry("User ID or invite link");
    container.append(&peer);

    let paste = pill_button("Paste");
    let peer_for_paste = peer.clone();
    paste.connect_clicked(move |_| {
        let entry = peer_for_paste.clone();
        clipboard::paste(move |value| {
            if !value.is_empty() {
                entry.set_text(&value);
            }
        });
    });
    container.append(&paste);

    let peer_for_scan = peer.clone();
    let scan = scan_qr_button("Scan QR", move |text| {
        peer_for_scan.set_text(&text);
    });
    container.append(&scan);

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

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.set_margin_top(16);
    separator.set_margin_bottom(8);
    container.append(&separator);

    let other_actions = adw::PreferencesGroup::new();

    let new_group = adw::ActionRow::builder()
        .title("New group")
        .subtitle("Create a multi-person chat")
        .activatable(true)
        .build();
    let chevron1 = gtk::Image::from_icon_name("go-next-symbolic");
    chevron1.add_css_class("dim-label");
    new_group.add_suffix(&chevron1);
    {
        let manager = manager.clone();
        new_group.connect_activated(move |_| {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::NewGroup,
            });
        });
    }
    other_actions.add(&new_group);

    let create_invite = adw::ActionRow::builder()
        .title("Share an invite link")
        .subtitle("Anyone with the link can chat with you")
        .activatable(true)
        .build();
    let chevron2 = gtk::Image::from_icon_name("go-next-symbolic");
    chevron2.add_css_class("dim-label");
    create_invite.add_suffix(&chevron2);
    {
        let manager = manager.clone();
        create_invite.connect_activated(move |_| {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::CreateInvite,
            });
        });
    }
    other_actions.add(&create_invite);

    let join_invite = adw::ActionRow::builder()
        .title("Join with invite link")
        .subtitle("Use a link someone shared with you")
        .activatable(true)
        .build();
    let chevron3 = gtk::Image::from_icon_name("go-next-symbolic");
    chevron3.add_css_class("dim-label");
    join_invite.add_suffix(&chevron3);
    {
        let manager = manager.clone();
        join_invite.connect_activated(move |_| {
            manager.dispatch(AppAction::PushScreen {
                screen: Screen::JoinInvite,
            });
        });
    }
    other_actions.add(&join_invite);

    container.append(&other_actions);

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
