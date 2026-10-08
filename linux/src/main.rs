mod app_manager;
mod calls;
mod platform;
mod screens;
mod secure_storage;
mod style;
mod widgets;
mod window;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "to.iris.chat";

fn main() -> glib::ExitCode {
    bootstrap_session_bus();
    let start_in_background = std::env::args().any(|arg| arg == platform::startup::BACKGROUND_ARG);

    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        style::install_css();
        gtk::Window::set_default_icon_name("iris-chat");
    });
    let manager = std::rc::Rc::new(std::cell::RefCell::new(
        None::<std::rc::Rc<app_manager::AppManager>>,
    ));
    let active_manager = manager.clone();
    app.connect_activate(move |app| {
        if let Some(manager) = window::build_ui(app, !start_in_background) {
            *active_manager.borrow_mut() = Some(manager);
        }
    });
    let weak_app = app.downgrade();
    platform::notifications::install_open_chat_action(&app, move |payload| {
        let Some(app) = weak_app.upgrade() else {
            return;
        };
        // Activation creates the manager on cold start and restores an existing window.
        app.activate();
        if let Some(manager) = manager.borrow().as_ref() {
            manager.receive_notification_chat(payload);
        }
        if let Some(window) = app
            .active_window()
            .or_else(|| app.windows().into_iter().next())
        {
            window.present();
        }
    });
    app.run()
}

// GApplication keys its single-instance behaviour off the session bus.
// Real Linux desktops always have one; in stripped-down environments
// (the dev container, sandboxes) shells often auto-launch a fresh bus
// each time, so two app launches each become their own primary. If we
// don't see a bus but the dev container has stood one up at the known
// path, point the process at it before GApplication registers.
fn bootstrap_session_bus() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return;
    }
    let socket = "/tmp/iris-dbus.sock";
    if std::path::Path::new(socket).exists() {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", socket));
    }
}
