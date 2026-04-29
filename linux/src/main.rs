mod app_manager;
mod platform;
mod screens;
mod secure_storage;
mod widgets;
mod window;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "to.iris.chat";

const CUSTOM_CSS: &str = r#"
.bubble-in,
.bubble-out {
    padding: 7px 12px;
    border-radius: 18px;
    min-height: 0;
}
.bubble-in {
    background-color: alpha(@view_fg_color, 0.08);
    color: @view_fg_color;
}
.bubble-out {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
}
.bubble-out .dim-label,
.bubble-in .dim-label {
    color: inherit;
    opacity: 0.72;
}
.bubble-meta {
    font-size: 0.78em;
    opacity: 0.72;
}
.chat-day {
    font-size: 0.8em;
    opacity: 0.55;
}
.chat-author {
    font-size: 0.78em;
    opacity: 0.65;
    margin-left: 12px;
}
"#;

fn main() -> glib::ExitCode {
    bootstrap_session_bus();
    let start_in_background = std::env::args().any(|arg| arg == platform::startup::BACKGROUND_ARG);

    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        install_css();
        gtk::Window::set_default_icon_name("iris-chat");
    });
    app.connect_activate(move |app| {
        window::build_ui(app, !start_in_background);
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

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CUSTOM_CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
