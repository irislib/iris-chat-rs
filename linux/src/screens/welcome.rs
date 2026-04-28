use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, Screen};

use crate::app_manager::AppManager;
use crate::screens::{dispatch_on_click, pill_button, primary_button, screen_container};

pub fn render(manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_spacing(18);
    container.set_width_request(320);

    let logo = logo_picture();
    logo.set_halign(gtk::Align::Center);
    logo.set_margin_bottom(4);
    container.append(&logo);

    let title = gtk::Label::new(None);
    title.set_markup("<span foreground=\"#702ACE\">iris</span> chat");
    title.add_css_class("title-1");
    title.set_margin_bottom(12);
    title.set_halign(gtk::Align::Center);
    container.append(&title);

    if iris_chat_core::is_trusted_test_build() {
        let banner = gtk::Label::new(Some("Test build"));
        banner.add_css_class("caption");
        banner.add_css_class("warning");
        banner.set_margin_bottom(8);
        container.append(&banner);
    }

    let create = welcome_button("Create account", "list-add-symbolic", true);
    dispatch_on_click(&create, manager, || AppAction::PushScreen {
        screen: Screen::CreateAccount,
    });
    container.append(&create);

    let restore = welcome_button("Restore account", "dialog-password-symbolic", false);
    dispatch_on_click(&restore, manager, || AppAction::PushScreen {
        screen: Screen::RestoreAccount,
    });
    container.append(&restore);

    let add_device = welcome_button("Link this device", "computer-symbolic", false);
    dispatch_on_click(&add_device, manager, || AppAction::PushScreen {
        screen: Screen::AddDevice,
    });
    container.append(&add_device);

    container.upcast()
}

fn logo_picture() -> gtk::Picture {
    let bytes = gtk::glib::Bytes::from_static(include_bytes!("../../resources/iris-chat-logo.png"));
    let texture = gtk::gdk::Texture::from_bytes(&bytes).expect("embedded Iris logo is valid PNG");
    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_size_request(132, 132);
    picture.set_can_shrink(true);
    picture
}

fn welcome_button(label: &str, icon_name: &str, primary: bool) -> gtk::Button {
    let button = if primary {
        primary_button(label)
    } else {
        pill_button(label)
    };
    button.set_halign(gtk::Align::Fill);
    button.set_width_request(320);

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(18);
    content.append(&icon);

    let text = gtk::Label::new(Some(label));
    content.append(&text);

    button.set_child(Some(&content));
    button
}
