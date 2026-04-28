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

    if state.link_device.is_none() && !state.busy.linking_device {
        manager.dispatch(AppAction::StartLinkedDevice {
            owner_input: String::new(),
        });
    }

    let header = gtk::Label::new(Some("Link this device"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let hint = gtk::Label::new(Some("Scan this code with your signed-in device."));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    container.append(&hint);

    if let Some(link) = state.link_device.as_ref() {
        container.append(&qr::build(&link.url, 240));

        let copy = primary_button("Copy link code");
        copy.set_halign(gtk::Align::Center);
        copy.set_width_request(220);
        let invite_url = link.url.clone();
        copy.connect_clicked(move |_| clipboard::copy(&invite_url));
        container.append(&copy);
    } else {
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_halign(gtk::Align::Center);
        container.append(&spinner);
    }

    let refresh = pill_button(if state.busy.linking_device {
        "Creating…"
    } else {
        "New code"
    });
    refresh.set_sensitive(!state.busy.linking_device);
    refresh.set_halign(gtk::Align::Center);
    refresh.set_width_request(220);
    {
        let manager = manager.clone();
        refresh.connect_clicked(move |_| {
            manager.dispatch(AppAction::StartLinkedDevice {
                owner_input: String::new(),
            });
        });
    }
    container.append(&refresh);

    container.upcast()
}
