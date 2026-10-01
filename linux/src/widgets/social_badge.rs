use adw::prelude::*;
use iris_chat_core::{SocialBadge, SocialConnectionSnapshot};

pub fn avatar(avatar: &adw::Avatar, connection: Option<&SocialConnectionSnapshot>) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    // Action rows can be taller than their avatar; anchor the overlay to the
    // avatar's natural bounds instead of placing the mark at the row's top.
    overlay.set_halign(gtk::Align::Center);
    overlay.set_valign(gtk::Align::Center);
    overlay.set_child(Some(avatar));
    if let Some(connection) = connection.filter(|c| c.badge.is_some()) {
        let badge = gtk::Label::new(Some(if connection.badge == Some(SocialBadge::Warning) {
            "!"
        } else if connection.badge == Some(SocialBadge::Muted) {
            "−"
        } else {
            "✓"
        }));
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.add_css_class("social-badge");
        badge.add_css_class(match connection.badge {
            Some(SocialBadge::Warning) => "warning",
            Some(SocialBadge::Following) => "following",
            Some(SocialBadge::Trusted) => "trusted",
            Some(SocialBadge::Muted) => "muted",
            _ => "friend",
        });
        badge.set_tooltip_text(Some(&connection.description));
        badge.update_property(&[gtk::accessible::Property::Label(&connection.description)]);
        overlay.add_overlay(&badge);
    }
    overlay
}

pub fn description(connection: &SocialConnectionSnapshot) -> gtk::Label {
    let label = gtk::Label::new(Some(&connection.description));
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label
}

pub const CSS: &str = r#"
.social-badge { border-radius: 50%; min-width: 16px; min-height: 16px; font-weight: bold; font-size: 11px; color: white; background: #8e8e93; }
.social-badge.following { background: #0a84ff; }
.social-badge.trusted { background: #d4a017; }
.social-badge.warning { background: @iris_accent_alt; }
.social-badge.muted { background: #c53030; }
"#;
