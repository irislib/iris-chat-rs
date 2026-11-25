use std::{cmp::Ordering, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use iris_chat_core::{AppAction, ChatKind, DesktopNearbyPeerSnapshot, DesktopNearbySnapshot};

use crate::app_manager::AppManager;
use crate::screens::chat::{chat_info_for_owner, present_chat_info, ChatInfoSnapshot};
use crate::widgets::clickable::PointerCursorExt;

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

    let mut rendered = None;
    update_list(&list, &status, &manager, &mut rendered);

    let list_for_updates = list.downgrade();
    let status_for_updates = status.downgrade();
    let lan_for_updates = lan.downgrade();
    let mailbag_for_updates = mailbag.downgrade();
    let manager_for_updates = manager.clone();
    glib::timeout_add_seconds_local(1, move || {
        // Stop once the dialog is gone.
        let (Some(list), Some(status), Some(lan), Some(mailbag)) = (
            list_for_updates.upgrade(),
            status_for_updates.upgrade(),
            lan_for_updates.upgrade(),
            mailbag_for_updates.upgrade(),
        ) else {
            return glib::ControlFlow::Break;
        };
        if list.root().is_none() {
            return glib::ControlFlow::Break;
        }
        let nearby_enabled = manager_for_updates
            .current_state()
            .preferences
            .nearby_enabled;
        lan.set_sensitive(nearby_enabled);
        mailbag.set_sensitive(nearby_enabled);
        update_list(&list, &status, &manager_for_updates, &mut rendered);
        glib::ControlFlow::Continue
    });

    // Scroll when many people are nearby; otherwise keep the natural height.
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    dialog.set_child(Some(&scroller));
    crate::widgets::dialogs::present(&dialog, parent);
}

type RenderedNearby = Option<(bool, DesktopNearbySnapshot)>;

/// Rebuilds rows only when Nearby changed, so rows and avatars stay put.
fn update_list(
    list: &gtk::ListBox,
    status: &gtk::Label,
    manager: &Rc<AppManager>,
    rendered: &mut RenderedNearby,
) {
    let next = (
        manager.current_state().preferences.nearby_enabled,
        manager.nearby_snapshot(),
    );
    if rendered.as_ref() == Some(&next) {
        return;
    }
    refresh(list, status, next.0, &next.1, manager);
    *rendered = Some(next);
}

#[cfg(feature = "ui-tests")]
pub(crate) fn nearby_list_for_test(
    manager: &Rc<AppManager>,
    rendered: &mut Option<(bool, DesktopNearbySnapshot)>,
) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    update_list(&list, &gtk::Label::new(None), manager, rendered);
    list
}

#[cfg(feature = "ui-tests")]
pub(crate) fn update_nearby_list_for_test(
    list: &gtk::ListBox,
    manager: &Rc<AppManager>,
    rendered: &mut Option<(bool, DesktopNearbySnapshot)>,
) {
    update_list(list, &gtk::Label::new(None), manager, rendered);
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
    if peer.owner_pubkey_hex.is_some() {
        row.show_pointer_cursor();
    }
    row.add_prefix(&peer_avatar(peer, &title, manager));
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

fn peer_avatar(
    peer: &DesktopNearbyPeerSnapshot,
    title: &str,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    const SIZE: i32 = 36;
    let avatar: gtk::Widget = match peer.owner_pubkey_hex.as_deref() {
        Some(owner) => crate::widgets::user_profile::avatar(manager, owner, SIZE, false).upcast(),
        None => {
            let avatar = adw::Avatar::new(SIZE, Some(title), true);
            if let Some(url) = peer.picture_url.as_deref() {
                crate::widgets::image_cache::fetch_proxied_into_avatar(
                    &avatar,
                    url,
                    &manager.current_state().preferences,
                    (SIZE * 2) as u32,
                );
            }
            avatar.upcast()
        }
    };
    avatar.set_valign(gtk::Align::Center);
    avatar
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
    chat_info_for_owner(
        owner,
        &name,
        peer.picture_url.as_deref(),
        &manager.current_state(),
    )
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
