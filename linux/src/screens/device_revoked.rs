use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::AppState;

use crate::app_manager::AppManager;

pub fn render(_state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let status = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("Device removed")
        .description("This device's access was removed. Sign in again to use Iris Chat here.")
        .build();
    status.set_vexpand(true);

    let acknowledge = gtk::Button::with_label("Sign in again");
    acknowledge.set_halign(gtk::Align::Center);
    acknowledge.add_css_class("suggested-action");
    acknowledge.add_css_class("pill");
    {
        let manager = manager.clone();
        acknowledge.connect_clicked(move |_| {
            manager.logout();
        });
    }
    status.set_child(Some(&acknowledge));

    status.upcast()
}
