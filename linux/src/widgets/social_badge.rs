use crate::app_manager::AppManager;
use adw::prelude::*;
use iris_chat_core::{SocialBadge, SocialConnectionSnapshot};

pub fn user_avatar(
    image: &adw::Avatar,
    connection: Option<&SocialConnectionSnapshot>,
    owner: &str,
    manager: &AppManager,
) -> gtk::Overlay {
    let overlay = avatar(image, connection);
    let size = (image.size() * 2 / 5).clamp(12, 24);
    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    badge.add_css_class("nearby-avatar-badge");
    // GTK adds the CSS border outside the requested content size.
    badge.set_size_request(size - 4, size - 4);
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    badge.set_tooltip_text(Some("Nearby"));
    badge.update_property(&[gtk::accessible::Property::Label("Nearby")]);
    let icon = gtk::Image::from_icon_name("network-wireless-symbolic");
    icon.set_pixel_size(size - 4);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_hexpand(true);
    badge.append(&icon);
    manager.track_nearby_avatar_badge(owner, badge.upcast_ref());
    overlay.add_overlay(&badge);
    overlay
}

pub fn avatar(avatar: &adw::Avatar, connection: Option<&SocialConnectionSnapshot>) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    // Action rows can be taller than their avatar; anchor the overlay to the
    // avatar's natural bounds instead of placing the mark at the row's top.
    overlay.set_halign(gtk::Align::Center);
    overlay.set_valign(gtk::Align::Center);
    overlay.set_child(Some(avatar));
    if connection.is_some_and(|connection| connection.is_favorite) {
        let favorite = gtk::Label::new(Some("★"));
        favorite.add_css_class("favorite-avatar-badge");
        favorite.set_halign(gtk::Align::Start);
        favorite.set_valign(gtk::Align::Start);
        favorite.set_tooltip_text(Some("Favorite · Only you"));
        favorite.update_property(&[gtk::accessible::Property::Label("Favorite")]);
        overlay.add_overlay(&favorite);
    }
    if let Some(badge) = connection.and_then(badge) {
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        overlay.add_overlay(&badge);
    }
    overlay
}

fn badge(connection: &SocialConnectionSnapshot) -> Option<gtk::Label> {
    connection.badge.as_ref()?;
    let badge = gtk::Label::new(Some(if connection.badge == Some(SocialBadge::Warning) {
        "!"
    } else if connection.badge == Some(SocialBadge::Muted) {
        "−"
    } else {
        "✓"
    }));
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
    Some(badge)
}

pub fn description(connection: &SocialConnectionSnapshot) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if let Some(badge) = badge(connection) {
        badge.set_valign(gtk::Align::Center);
        row.append(&badge);
    }
    let label = gtk::Label::new(Some(&connection.description));
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    row
}

pub const CSS: &str = r#"
.social-badge { border-radius: 50%; min-width: 16px; min-height: 16px; font-weight: bold; font-size: 11px; color: white; background: #8e8e93; }
.social-badge.following { background: #0a84ff; }
.social-badge.trusted { background: #d4a017; }
.social-badge.warning { background: @iris_accent_alt; }
.social-badge.muted { background: #c53030; }
.favorite-avatar-badge { border-radius: 50%; border: 1px solid @window_bg_color; min-width: 14px; min-height: 14px; font-size: 11px; color: #352900; background: #fbbf24; }
.nearby-avatar-badge { border-radius: 50%; border: 2px solid @window_bg_color; color: white; background: #2267f5; }
"#;
