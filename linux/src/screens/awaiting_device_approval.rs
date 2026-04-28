use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::AppState;

use crate::app_manager::AppManager;
use crate::screens::confirm_delete_app_data;

pub fn render(_state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let status = adw::StatusPage::builder()
        .icon_name("emblem-synchronizing-symbolic")
        .title("Waiting for approval")
        .description("Use your signed-in device to approve this one.")
        .build();
    status.set_vexpand(true);

    let cancel = gtk::Button::with_label("Sign out");
    cancel.set_halign(gtk::Align::Center);
    cancel.add_css_class("pill");
    {
        let manager = manager.clone();
        cancel.connect_clicked(move |button| {
            let parent = button.root().and_then(|root| root.downcast::<gtk::Window>().ok());
            confirm_delete_app_data(parent.as_ref(), &manager);
        });
    }
    status.set_child(Some(&cancel));

    status.upcast()
}
