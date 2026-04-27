use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState};

use crate::app_manager::AppManager;

pub fn render(_state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let status = adw::StatusPage::builder()
        .icon_name("emblem-synchronizing-symbolic")
        .title("Waiting for approval")
        .description("Approve this device from another signed-in device with owner authority.")
        .build();
    status.set_vexpand(true);

    let cancel = gtk::Button::with_label("Sign out");
    cancel.set_halign(gtk::Align::Center);
    cancel.add_css_class("pill");
    {
        let manager = manager.clone();
        cancel.connect_clicked(move |_| {
            manager.dispatch(AppAction::Logout);
        });
    }
    status.set_child(Some(&cancel));

    status.upcast()
}
