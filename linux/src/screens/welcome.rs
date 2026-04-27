use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, Screen};

use crate::app_manager::AppManager;
use crate::screens::{dispatch_on_click, pill_button, primary_button, screen_container};

pub fn render(manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();
    container.set_valign(gtk::Align::Center);
    container.set_spacing(18);

    let title = gtk::Label::new(Some("Iris Chat"));
    title.add_css_class("title-1");
    title.set_margin_bottom(12);
    container.append(&title);

    let tagline = gtk::Label::new(Some("End-to-end encrypted chat over Nostr"));
    tagline.add_css_class("dim-label");
    tagline.set_margin_bottom(12);
    container.append(&tagline);

    if ndr_demo_core::is_trusted_test_build() {
        let banner = gtk::Label::new(Some("Trusted test build"));
        banner.add_css_class("caption");
        banner.add_css_class("warning");
        banner.set_margin_bottom(8);
        container.append(&banner);
    }

    let create = primary_button("Create account");
    dispatch_on_click(&create, manager, || AppAction::PushScreen {
        screen: Screen::CreateAccount,
    });
    container.append(&create);

    let restore = pill_button("Restore account");
    dispatch_on_click(&restore, manager, || AppAction::PushScreen {
        screen: Screen::RestoreAccount,
    });
    container.append(&restore);

    let add_device = pill_button("Add this device");
    dispatch_on_click(&add_device, manager, || AppAction::PushScreen {
        screen: Screen::AddDevice,
    });
    container.append(&add_device);

    container.upcast()
}
