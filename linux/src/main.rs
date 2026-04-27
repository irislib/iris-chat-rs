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
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_startup(|_| install_css());
    app.connect_activate(window::build_ui);
    app.run()
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
