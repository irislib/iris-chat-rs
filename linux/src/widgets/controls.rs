use gtk::prelude::*;

/// Keep icon controls square when the composer grows to several lines.
pub fn composer_control(control: &impl IsA<gtk::Widget>) {
    control.add_css_class("composer-control");
    control.add_css_class("circular");
    control.set_valign(gtk::Align::Center);
}

pub const CSS: &str = r#"
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
