mod app_manager;
mod platform;
mod screens;
mod secure_storage;
mod widgets;
mod window;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "to.iris.chat";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_activate(window::build_ui);
    app.run()
}
