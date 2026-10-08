use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, ChatKind, DesktopNearbyPeerSnapshot};

use crate::app_manager::AppManager;
use crate::screens::chat::{present_chat_info, ChatInfoSnapshot};
use crate::widgets::clickable::PointerCursorExt;
use crate::widgets::image_cache;

pub(super) fn nearby_row(manager: &Rc<AppManager>) -> gtk::Widget {
    let snapshot = manager.nearby_snapshot();
    let nearby_enabled = manager.current_state().preferences.nearby_enabled;
    let active = nearby_enabled && snapshot.visible;
    let peers = if nearby_enabled {
        snapshot.peers.as_slice()
    } else {
        &[]
    };
    const NEARBY_AVATAR_SIZE: i32 = 40;
    const NEARBY_ROW_CONTENT_HEIGHT: i32 = NEARBY_AVATAR_SIZE + 22;

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_hexpand(true);
    outer.set_size_request(-1, NEARBY_ROW_CONTENT_HEIGHT);
    outer.set_valign(gtk::Align::Start);

    if !peers.is_empty() {
        outer.append(&nearby_icon_button(manager, active, NEARBY_AVATAR_SIZE));
        outer.append(&nearby_avatar_strip(peers, manager));
        return outer.upcast();
    }

    outer.append(&nearby_icon(active, NEARBY_AVATAR_SIZE));

    let label = gtk::Label::new(Some(if !nearby_enabled {
        "Off"
    } else if active {
        "No users nearby"
    } else {
        "Tap to enable"
    }));
    label.set_valign(gtk::Align::Center);
    label.set_halign(gtk::Align::Start);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("dim-label");
    label.set_hexpand(true);
    outer.append(&label);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.show_pointer_cursor();
    button.set_child(Some(&outer));
    let manager_for_click = manager.clone();
    button.connect_clicked(move |btn| {
        present_nearby_from_button(btn, manager_for_click.clone());
    });
    button.upcast()
}

fn nearby_icon_button(manager: &Rc<AppManager>, active: bool, size: i32) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("nearby-avatar-button");
    button.show_pointer_cursor();
    button.set_size_request(size, size);
    button.set_tooltip_text(Some("Nearby"));
    button.set_child(Some(&nearby_icon(active, size)));
    button.set_valign(gtk::Align::Start);
    let manager_for_click = manager.clone();
    button.connect_clicked(move |btn| {
        present_nearby_from_button(btn, manager_for_click.clone());
    });
    button
}

fn nearby_icon(active: bool, size: i32) -> gtk::Box {
    let background = gtk::Box::new(gtk::Orientation::Vertical, 0);
    background.set_size_request(size, size);
    background.set_valign(gtk::Align::Start);
    background.set_halign(gtk::Align::Center);
    background.add_css_class("nearby-avatar");
    background.add_css_class(if active {
        "nearby-active"
    } else {
        "nearby-off"
    });
    let icon = gtk::Image::from_icon_name("network-wireless-symbolic");
    icon.set_pixel_size(24);
    icon.set_vexpand(true);
    icon.set_valign(gtk::Align::Center);
    icon.set_halign(gtk::Align::Center);
    icon.add_css_class(if active {
        "nearby-active-icon"
    } else {
        "dim-label"
    });
    background.append(&icon);
    background
}

fn present_nearby_from_button(button: &gtk::Button, manager: Rc<AppManager>) {
    let parent = button.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    crate::screens::present_nearby(parent.as_ref(), manager);
}

fn nearby_avatar_strip(
    peers: &[DesktopNearbyPeerSnapshot],
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let avatar_size: i32 = 40;
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    strip.set_valign(gtk::Align::Start);
    let prefs = manager.current_state().preferences.clone();
    for peer in peers {
        let name = nearby_peer_resolved_name(peer, manager, "Nearby user");
        let avatar = adw::Avatar::new(avatar_size, Some(&name), true);
        if let Some(url) = peer.picture_url.as_ref() {
            image_cache::fetch_proxied_into_avatar(&avatar, url, &prefs, (avatar_size * 2) as u32);
        }
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_size_request(64, -1);
        column.set_halign(gtk::Align::Center);
        column.set_valign(gtk::Align::Start);
        column.append(&crate::widgets::social_badge::user_avatar(
            &avatar,
            manager
                .contact_social_connection(peer.owner_pubkey_hex.as_deref().unwrap_or_default())
                .as_ref(),
            peer.owner_pubkey_hex.as_deref().unwrap_or_default(),
            manager,
        ));
        let label = gtk::Label::new(Some(&nearby_peer_display_name(&name)));
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_max_width_chars(9);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_halign(gtk::Align::Center);
        label.set_xalign(0.5);
        column.append(&label);

        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.show_pointer_cursor();
        button.set_child(Some(&column));
        button.set_tooltip_text(Some(&name));
        if let Some(owner) = peer.owner_pubkey_hex.clone() {
            let manager_for_click = manager.clone();
            let peer_for_click = peer.clone();
            button.connect_clicked(move |button| {
                open_nearby_peer_from_widget(
                    button,
                    &peer_for_click,
                    owner.as_str(),
                    manager_for_click.clone(),
                );
            });
        } else {
            button.set_sensitive(false);
        }
        strip.append(&button);
    }

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(avatar_size + 22);
    scrolled.set_child(Some(&strip));
    scrolled.upcast()
}

fn nearby_peer_display_name(name: &str) -> String {
    let trimmed = name.trim();
    let value = if trimmed.is_empty() {
        "Nearby"
    } else {
        trimmed
    };
    if value.chars().count() <= 14 {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(13).collect::<String>())
    }
}

fn nearby_peer_resolved_name(
    peer: &DesktopNearbyPeerSnapshot,
    manager: &Rc<AppManager>,
    fallback: &str,
) -> String {
    if let Some(owner) = peer.owner_pubkey_hex.as_deref() {
        let state = manager.current_state();
        if let Some(chat) = state.chat_list.iter().find(|chat| {
            matches!(chat.kind, ChatKind::Direct) && chat.chat_id.eq_ignore_ascii_case(owner)
        }) {
            let name = chat.display_name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    let name = peer.name.trim();
    if name.is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    }
}

fn open_nearby_peer_from_widget(
    widget: &gtk::Button,
    peer: &DesktopNearbyPeerSnapshot,
    owner: &str,
    manager: Rc<AppManager>,
) {
    if is_known_direct_chat(&manager, owner) {
        manager.dispatch(AppAction::OpenChat {
            chat_id: owner.to_string(),
        });
        return;
    }

    let parent = widget.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    present_chat_info(
        parent.as_ref(),
        nearby_peer_chat_info(peer, owner, &manager),
        manager,
    );
}

fn is_known_direct_chat(manager: &Rc<AppManager>, owner: &str) -> bool {
    manager.current_state().chat_list.iter().any(|chat| {
        matches!(chat.kind, ChatKind::Direct) && chat.chat_id.eq_ignore_ascii_case(owner)
    })
}

fn nearby_peer_chat_info(
    peer: &DesktopNearbyPeerSnapshot,
    owner: &str,
    manager: &Rc<AppManager>,
) -> ChatInfoSnapshot {
    let name = nearby_peer_resolved_name(peer, manager, "Nearby user");
    ChatInfoSnapshot {
        social_connection: None,
        chat_id: owner.to_string(),
        display_name: name,
        nickname: None,
        contact_note: None,
        profile_name: None,
        subtitle: None,
        picture_url: peer.picture_url.clone(),
        about: None,
        is_muted: false,
        show_message_action: true,
        preferences: manager.current_state().preferences,
    }
}
