use super::clickable::PointerCursorExt;
use gtk::prelude::*;

/// Keep icon controls square when the composer grows to several lines.
pub fn composer_control(control: &impl IsA<gtk::Widget>) {
    control.add_css_class("composer-control");
    control.add_css_class("circular");
    control.set_valign(gtk::Align::Center);
    control.show_pointer_cursor();
}

/// Filled paper plane for send buttons. Bundled so it reads the same in
/// every icon theme (several themes draw "send" as an upload arrow).
pub fn send_icon() -> gtk::Image {
    thread_local! {
        static TEXTURE: gtk::gdk::Texture = gtk::gdk::Texture::from_bytes(
            &gtk::glib::Bytes::from_static(include_bytes!("../../resources/send.png")),
        )
        .expect("bundled send icon");
    }
    let image = TEXTURE.with(|texture| gtk::Image::from_paintable(Some(texture)));
    image.set_pixel_size(20);
    // The plane's weight sits left of its box; nudge it to look centered.
    image.set_margin_start(2);
    image
}

/// Circular header back button.
pub fn back_button() -> gtk::Button {
    let button = gtk::Button::from_icon_name("go-previous-symbolic");
    button.add_css_class("circular");
    button.add_css_class("header-back");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some("Back"));
    button
}

pub const CSS: &str = r#"
headerbar button.header-back {
    border-radius: 9999px;
    min-width: 34px;
    min-height: 34px;
    padding: 0;
}
button.composer-control.circular,
menubutton.composer-control > button {
    border-radius: 9999px;
    min-width: 40px;
    min-height: 40px;
    padding: 0;
}
.nearby-avatar {
    border-radius: 9999px;
}
button.nearby-avatar-button {
    border-radius: 9999px;
    min-width: 0;
    min-height: 0;
    padding: 0;
}
button.nearby-preview-button {
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: 0;
}
button.reaction-chip {
    border-radius: 9999px;
    min-width: 0;
    min-height: 22px;
    padding: 0 7px;
    background: @iris_panel_alt;
    border: 1px solid @iris_background;
    color: @iris_text_primary;
    box-shadow: none;
}
button.reaction-chip:hover {
    background: mix(@iris_panel_alt, @iris_text_primary, 0.08);
}
button.reaction-chip.reaction-selected {
    background: @iris_panel;
    border-color: alpha(@iris_muted, 0.32);
}
button.reaction-chip.reaction-selected:hover {
    background: mix(@iris_panel, @iris_text_primary, 0.08);
}
.reaction-emoji {
    font-size: 14px;
}
.reaction-count {
    font-size: 12px;
    font-weight: 600;
    font-family: monospace;
    color: @iris_muted;
}
"#;
