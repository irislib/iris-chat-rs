use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState};

use crate::app_manager::AppManager;
use crate::platform::clipboard;
use crate::screens::{pill_button, primary_button, screen_container};
use crate::widgets::qr;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();
    container.set_spacing(16);

    if state.public_invite.is_none() && !state.busy.creating_invite {
        manager.dispatch(AppAction::CreatePublicInvite);
    }

    let title = gtk::Label::new(Some("Invite"));
    title.add_css_class("title-2");
    title.set_halign(gtk::Align::Start);
    container.append(&title);

    if let Some(invite) = state.public_invite.as_ref() {
        container.append(&qr::build(&invite.url, 240));
        let open_on_join = gtk::CheckButton::with_label("Open chat when someone joins");
        open_on_join.set_widget_name("inviteOpenOnJoinToggle");
        open_on_join.set_active(invite.open_chat_on_join);
        let manager = manager.clone();
        open_on_join.connect_toggled(move |button| {
            manager.dispatch(AppAction::SetInviteOpenOnJoin {
                enabled: button.is_active(),
            })
        });
        container.append(&open_on_join);

        let copy = primary_button("Copy");
        copy.set_halign(gtk::Align::Center);
        copy.set_width_request(220);
        let invite_url = invite.url.clone();
        copy.connect_clicked(move |_| clipboard::copy(&invite_url));
        container.append(&copy);
    } else {
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_halign(gtk::Align::Center);
        container.append(&spinner);
    }

    let refresh = pill_button(if state.busy.creating_invite {
        "Creating…"
    } else {
        "New invite"
    });
    refresh.set_sensitive(!state.busy.creating_invite);
    refresh.set_halign(gtk::Align::Center);
    refresh.set_width_request(220);
    {
        let manager = manager.clone();
        refresh.connect_clicked(move |_| {
            manager.dispatch(AppAction::CreatePublicInvite);
        });
    }
    container.append(&refresh);

    container.upcast()
}
