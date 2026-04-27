use adw::prelude::*;
use gtk::gio;

pub fn notify(app_id: &str, title: &str, body: &str) {
    let Some(app) = gio::Application::default() else {
        return;
    };
    let notification = gio::Notification::new(title);
    notification.set_body(Some(body));
    app.send_notification(Some(app_id), &notification);
}
