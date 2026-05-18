mod app_manager;
mod platform;
mod screens;
mod secure_storage;
mod widgets;
mod window;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "to.iris.chat";

#[derive(Clone, Copy)]
struct GtkPalette {
    background: &'static str,
    panel: &'static str,
    panel_alt: &'static str,
    border: &'static str,
    toolbar: &'static str,
    bubble_mine: &'static str,
    bubble_theirs: &'static str,
    accent: &'static str,
    accent_alt: &'static str,
    text_primary: &'static str,
    muted: &'static str,
    on_accent: &'static str,
    on_bubble_mine: &'static str,
    on_bubble_theirs: &'static str,
}

const IRIS_LIGHT: GtkPalette = GtkPalette {
    background: "#FFFFFF",
    panel: "#F7F9FA",
    panel_alt: "#E1E8ED",
    border: "rgba(0, 0, 0, 0.08)",
    toolbar: "rgba(247, 249, 250, 0.96)",
    bubble_mine: "#702ACE",
    bubble_theirs: "#F7F9FA",
    accent: "#702ACE",
    accent_alt: "#DB8216",
    text_primary: "#0F1419",
    muted: "#536471",
    on_accent: "#FFFFFF",
    on_bubble_mine: "#FFFFFF",
    on_bubble_theirs: "#0F1419",
};

const IRIS_DARK: GtkPalette = GtkPalette {
    background: "#000000",
    panel: "#161616",
    panel_alt: "#262626",
    border: "rgba(255, 255, 255, 0.12)",
    toolbar: "rgba(10, 10, 10, 0.96)",
    bubble_mine: "#702ACE",
    bubble_theirs: "#3A3A3A",
    accent: "#702ACE",
    accent_alt: "#DB8216",
    text_primary: "#FFFFFF",
    muted: "#D1D5DB",
    on_accent: "#FFFFFF",
    on_bubble_mine: "#FFFFFF",
    on_bubble_theirs: "#FFFFFF",
};

const CUSTOM_CSS: &str = r#"
.bubble-in,
.bubble-out {
    padding: 7px 12px;
    border-radius: 18px;
    min-height: 0;
}
.bubble-in {
    background-color: @iris_bubble_theirs;
    color: @iris_on_bubble_theirs;
}
.bubble-out {
    background-color: @iris_bubble_mine;
    color: @iris_on_bubble_mine;
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
.bubble-jumbomoji-1 {
    font-size: 56px;
    line-height: 1.05;
}
.bubble-jumbomoji-2 {
    font-size: 48px;
    line-height: 1.05;
}
.bubble-jumbomoji-3 {
    font-size: 40px;
    line-height: 1.05;
}
.bubble-jumbomoji-4 {
    font-size: 36px;
    line-height: 1.05;
}
.bubble-jumbomoji-5 {
    font-size: 32px;
    line-height: 1.05;
}
/* Show more/less toggle inside a bubble. We dropped Adwaita's
 * `.link` class to keep the toggle from rendering in brand purple
 * (which violates the "no purple text/icons" rule), so we have to
 * paint it ourselves. `color: inherit` picks up the bubble's
 * on-bubble foreground; opacity matches `.bubble-meta` so the
 * toggle reads as a quiet affordance attached to the message. */
.bubble-toggle {
    color: inherit;
    background: transparent;
    box-shadow: none;
    padding: 2px 4px;
    margin-top: 2px;
    font-size: 0.82em;
    font-weight: 600;
    opacity: 0.85;
}
.bubble-toggle:hover {
    background: transparent;
    opacity: 1.0;
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
    let style_manager = adw::StyleManager::default();
    load_iris_css(&provider, style_manager.is_dark());
    {
        let provider = provider.clone();
        style_manager.connect_dark_notify(move |manager| {
            load_iris_css(&provider, manager.is_dark());
        });
    }
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn load_iris_css(provider: &gtk::CssProvider, dark: bool) {
    let palette = if dark { IRIS_DARK } else { IRIS_LIGHT };
    let css = format!("{}\n{}", palette_css(palette), CUSTOM_CSS);
    provider.load_from_string(&css);
}

fn palette_css(palette: GtkPalette) -> String {
    format!(
        r#"
@define-color iris_background {background};
@define-color iris_panel {panel};
@define-color iris_panel_alt {panel_alt};
@define-color iris_border {border};
@define-color iris_toolbar {toolbar};
@define-color iris_bubble_mine {bubble_mine};
@define-color iris_bubble_theirs {bubble_theirs};
@define-color iris_accent {accent};
@define-color iris_accent_alt {accent_alt};
@define-color iris_text_primary {text_primary};
@define-color iris_muted {muted};
@define-color iris_on_accent {on_accent};
@define-color iris_on_bubble_mine {on_bubble_mine};
@define-color iris_on_bubble_theirs {on_bubble_theirs};

@define-color window_bg_color {background};
@define-color window_fg_color {text_primary};
@define-color view_bg_color {background};
@define-color view_fg_color {text_primary};
@define-color headerbar_bg_color {toolbar};
@define-color headerbar_fg_color {text_primary};
@define-color card_bg_color {panel};
@define-color card_fg_color {text_primary};
@define-color accent_bg_color {accent};
@define-color accent_fg_color {on_accent};
@define-color accent_color {accent};

.iris-root,
.iris-root viewport,
.iris-root scrolledwindow,
window {{
    background-color: @iris_background;
    color: @iris_text_primary;
}}

headerbar {{
    background-color: @iris_toolbar;
    color: @iris_text_primary;
    box-shadow: inset 0 -1px @iris_border;
}}

entry {{
    background-color: @iris_panel_alt;
    color: @iris_text_primary;
    border-color: @iris_border;
}}

entry:focus {{
    border-color: @iris_accent;
}}

textview.composer-input {{
    background-color: @iris_panel_alt;
    color: @iris_text_primary;
    border: 1px solid @iris_border;
    border-radius: 20px;
}}

textview.composer-input:focus {{
    border-color: @iris_accent;
}}

textview.composer-input text {{
    background-color: transparent;
    color: @iris_text_primary;
}}

button.flat {{
    background-color: transparent;
    color: @iris_text_primary;
    border-color: transparent;
}}

button.pill {{
    background-color: @iris_panel_alt;
    color: @iris_text_primary;
    border: 2px solid @iris_background;
}}

button.suggested-action {{
    background-color: @iris_accent;
    color: @iris_on_accent;
    border-color: @iris_accent;
}}

.card,
.boxed-list {{
    background-color: @iris_panel;
    color: @iris_text_primary;
    border-color: @iris_border;
}}

/* Signal-style chat list: rows blend with the panel, no hairlines. */
.boxed-list > row + row {{
    border-top: none;
}}

.dim-label {{
    color: @iris_muted;
}}

.accent {{
    color: @iris_accent;
}}

.nearby-active {{
    background-color: #2267F5;
    color: #fff;
}}

.nearby-active-icon {{
    color: #fff;
}}

.nearby-off {{
    background-color: @iris_panel_alt;
    color: @iris_muted;
}}

.warning {{
    background-color: @iris_accent_alt;
    color: @iris_on_accent;
}}
"#,
        background = palette.background,
        panel = palette.panel,
        panel_alt = palette.panel_alt,
        border = palette.border,
        toolbar = palette.toolbar,
        bubble_mine = palette.bubble_mine,
        bubble_theirs = palette.bubble_theirs,
        accent = palette.accent,
        accent_alt = palette.accent_alt,
        text_primary = palette.text_primary,
        muted = palette.muted,
        on_accent = palette.on_accent,
        on_bubble_mine = palette.on_bubble_mine,
        on_bubble_theirs = palette.on_bubble_theirs,
    )
}
