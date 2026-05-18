use std::{cmp::Ordering, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use iris_chat_core::{AppAction, ChatKind, DesktopNearbyPeerSnapshot, DesktopNearbySnapshot};

use crate::app_manager::AppManager;
use crate::screens::chat::{present_chat_info, ChatInfoSnapshot};

pub fn present(parent: Option<&gtk::Window>, manager: Rc<AppManager>) {
    let dialog = adw::Dialog::builder()
        .title("Nearby")
        .content_width(360)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let prefs = manager.current_state().preferences;

    let master = adw::SwitchRow::builder().title("Nearby").build();
    master.set_active(prefs.nearby_enabled);
    {
        let manager = manager.clone();
        master.connect_active_notify(move |row| {
            manager.set_nearby_enabled(row.is_active());
        });
    }
    content.append(&master);

    let lan = adw::SwitchRow::builder().title("Wi-Fi").build();
    lan.set_active(prefs.nearby_lan_enabled);
    lan.set_sensitive(prefs.nearby_enabled);
    {
        let manager = manager.clone();
        lan.connect_active_notify(move |row| {
            manager.set_nearby_lan_enabled(row.is_active());
        });
    }
    content.append(&lan);

    // Mirrors the Wi-Fi row so Mailbag reads as another transport-
    // layer thing the user can pause without losing the bag's
    // existing contents. The subtitle calls out that this carries
    // other people's messages too.
    let mailbag = adw::SwitchRow::builder()
        .title("Mailbag")
        .subtitle(
            "Anonymously carries messages by you and others over Bluetooth or Wi-Fi, \
             so they keep moving where there's no internet.",
        )
        .build();
    mailbag.set_active(prefs.nearby_mailbag_enabled);
    mailbag.set_sensitive(prefs.nearby_enabled);
    {
        let manager = manager.clone();
        mailbag.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetNearbyMailbagEnabled {
                enabled: row.is_active(),
            });
        });
    }
    content.append(&mailbag);

    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    content.append(&status);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    content.append(&list);

    refresh(
        &list,
        &status,
        manager.current_state().preferences.nearby_enabled,
        &manager.nearby_snapshot(),
        &manager,
    );

    let list_for_updates = list.clone();
    let status_for_updates = status.clone();
    let lan_for_updates = lan.clone();
    let mailbag_for_updates = mailbag.clone();
    let manager_for_updates = manager.clone();
    glib::timeout_add_seconds_local(1, move || {
        let snapshot = manager_for_updates.nearby_snapshot();
        let nearby_enabled = manager_for_updates
            .current_state()
            .preferences
            .nearby_enabled;
        lan_for_updates.set_sensitive(nearby_enabled);
        mailbag_for_updates.set_sensitive(nearby_enabled);
        refresh(
            &list_for_updates,
            &status_for_updates,
            nearby_enabled,
            &snapshot,
            &manager_for_updates,
        );
        glib::ControlFlow::Continue
    });

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn refresh(
    list: &gtk::ListBox,
    status: &gtk::Label,
    nearby_enabled: bool,
    snapshot: &DesktopNearbySnapshot,
    manager: &Rc<AppManager>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let status_text = if nearby_enabled
        && snapshot.visible
        && is_wifi_blocking_status(snapshot.status.as_str())
    {
        wifi_status_label(snapshot.status.as_str())
    } else {
        String::new()
    };
    status.set_label(&status_text);
    status.set_visible(!status_text.is_empty());

    let mut peers: Vec<DesktopNearbyPeerSnapshot> = if nearby_enabled {
        snapshot.peers.clone()
    } else {
        Vec::new()
    };
    peers.sort_by(|left, right| compare_nearby_peers(left, right, manager));

    if peers.is_empty() {
        let row = adw::ActionRow::builder()
            .title(if nearby_enabled {
                "No users nearby"
            } else {
                "Off"
            })
            .sensitive(false)
            .build();
        list.append(&row);
        return;
    }

    for peer in &peers {
        list.append(&peer_row(peer, manager));
    }
}

fn wifi_status_label(status: &str) -> String {
    match status {
        "Local network unavailable" => "Wi-Fi unavailable".to_string(),
        "Local network failed" => "Wi-Fi failed".to_string(),
        "No local network access" => "No Wi-Fi access".to_string(),
        _ => status.to_string(),
    }
}

fn is_wifi_blocking_status(status: &str) -> bool {
    matches!(
        status,
        "Local network unavailable" | "Local network failed" | "No local network access"
    )
}

fn peer_row(peer: &DesktopNearbyPeerSnapshot, manager: &Rc<AppManager>) -> adw::ActionRow {
    let title = nearby_peer_resolved_name(peer, manager, "Nearby user");
    let row = adw::ActionRow::builder()
        .title(title.as_str())
        .activatable(peer.owner_pubkey_hex.is_some())
        .build();
    let icon = gtk::Image::from_icon_name("avatar-default-symbolic");
    row.add_prefix(&icon);
    if let Some(owner) = peer.owner_pubkey_hex.clone() {
        let manager = manager.clone();
        let peer = peer.clone();
        row.connect_activated(move |row| {
            if is_known_direct_chat(&manager, owner.as_str()) {
                manager.dispatch(AppAction::OpenChat {
                    chat_id: owner.clone(),
                });
                return;
            }

            let parent = row.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            present_chat_info(
                parent.as_ref(),
                nearby_peer_chat_info(&peer, owner.as_str(), &manager),
                manager.clone(),
            );
        });
    }
    row
}

fn is_known_direct_chat(manager: &Rc<AppManager>, owner: &str) -> bool {
    manager.current_state().chat_list.iter().any(|chat| {
        matches!(chat.kind, ChatKind::Direct) && chat.chat_id.eq_ignore_ascii_case(owner)
    })
}

fn compare_nearby_peers(
    left: &DesktopNearbyPeerSnapshot,
    right: &DesktopNearbyPeerSnapshot,
    manager: &Rc<AppManager>,
) -> Ordering {
    let left_has_chat = left
        .owner_pubkey_hex
        .as_deref()
        .is_some_and(|owner| is_known_direct_chat(manager, owner));
    let right_has_chat = right
        .owner_pubkey_hex
        .as_deref()
        .is_some_and(|owner| is_known_direct_chat(manager, owner));
    left_has_chat
        .cmp(&right_has_chat)
        .reverse()
        .then_with(|| {
            deterministic_nearby_peer_key(left).cmp(&deterministic_nearby_peer_key(right))
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn deterministic_nearby_peer_key(peer: &DesktopNearbyPeerSnapshot) -> String {
    peer.owner_pubkey_hex
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .map(str::to_lowercase)
        .unwrap_or_else(|| format!("peer:{}", peer.id.to_lowercase()))
}

fn nearby_peer_chat_info(
    peer: &DesktopNearbyPeerSnapshot,
    owner: &str,
    manager: &Rc<AppManager>,
) -> ChatInfoSnapshot {
    let name = nearby_peer_resolved_name(peer, manager, "Nearby user");
    ChatInfoSnapshot {
        chat_id: owner.to_string(),
        display_name: name,
        nickname: None,
        profile_name: None,
        subtitle: None,
        picture_url: peer.picture_url.clone(),
        about: None,
        is_muted: false,
        show_message_action: true,
        preferences: manager.current_state().preferences,
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
