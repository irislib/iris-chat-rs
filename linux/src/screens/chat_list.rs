use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use iris_chat_core::{
    proxied_image_url, AppAction, AppState, ChatThreadSnapshot, DesktopNearbyPeerSnapshot,
    PreferencesSnapshot,
};

use crate::app_manager::AppManager;
use crate::widgets::image_cache;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    list.set_margin_start(12);
    list.set_margin_end(12);

    let now = unix_now();
    list.append(&nearby_row(manager));
    for chat in &state.chat_list {
        list.append(&row_for(chat, &state.preferences, now, manager));
    }

    scrolled.set_child(Some(&list));
    scrolled.upcast()
}

fn nearby_row(manager: &Rc<AppManager>) -> gtk::Widget {
    let snapshot = manager.nearby_snapshot();
    let subtitle = if !snapshot.visible {
        "Click to enable".to_string()
    } else if !snapshot.peers.is_empty() {
        nearby_summary(&snapshot.peers)
    } else {
        wifi_status_label(&snapshot.status)
    };

    // Custom row instead of adw::ActionRow because we need an inline
    // avatar stack rendered next to the subtitle text — the avatars
    // belong on the same line as the "Boromir nearby" label, not in
    // the prefix slot where they'd replace the wireless icon.
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);
    outer.set_margin_start(12);
    outer.set_margin_end(12);

    let icon = gtk::Image::from_icon_name("network-wireless-symbolic");
    icon.set_pixel_size(24);
    icon.set_valign(gtk::Align::Center);
    outer.append(&icon);

    let text_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_col.set_valign(gtk::Align::Center);
    text_col.set_hexpand(true);
    let title_label = gtk::Label::new(Some("Nearby"));
    title_label.set_halign(gtk::Align::Start);
    title_label.set_xalign(0.0);
    title_label.add_css_class("heading");
    text_col.append(&title_label);

    let subtitle_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    subtitle_row.set_halign(gtk::Align::Start);
    if !snapshot.peers.is_empty() {
        subtitle_row.append(&nearby_avatar_stack(&snapshot.peers, manager));
    }
    let subtitle_label = gtk::Label::new(Some(&subtitle));
    subtitle_label.set_halign(gtk::Align::Start);
    subtitle_label.set_xalign(0.0);
    subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle_label.add_css_class("dim-label");
    subtitle_label.add_css_class("caption");
    subtitle_row.append(&subtitle_label);
    text_col.append(&subtitle_row);

    outer.append(&text_col);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.set_child(Some(&outer));
    let manager_for_click = manager.clone();
    button.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        crate::screens::present_nearby(parent.as_ref(), manager_for_click.clone());
    });
    button.upcast()
}

fn nearby_avatar_stack(
    peers: &[DesktopNearbyPeerSnapshot],
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let take = peers.len().min(3);
    let avatar_size: i32 = 16;
    let overlap: i32 = 6;
    let stride = avatar_size - overlap;
    let stack_width = stride * (take as i32 - 1) + avatar_size;
    let overlay = gtk::Fixed::new();
    overlay.set_size_request(stack_width, avatar_size);
    overlay.set_valign(gtk::Align::Center);
    let prefs = manager.current_state().preferences.clone();
    for (index, peer) in peers.iter().take(take).enumerate() {
        let avatar = adw::Avatar::new(avatar_size, Some(&peer.name), true);
        if let Some(url) = peer.picture_url.as_ref() {
            let proxied = proxied_image_url(
                url.clone(),
                prefs.clone(),
                Some((avatar_size * 2) as u32),
                Some((avatar_size * 2) as u32),
                true,
            );
            image_cache::fetch_into_avatar(&avatar, &proxied);
        }
        overlay.put(&avatar, (stride * index as i32) as f64, 0.0);
    }
    overlay.upcast()
}

fn row_for(
    chat: &ChatThreadSnapshot,
    prefs: &PreferencesSnapshot,
    now: u64,
    manager: &Rc<AppManager>,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(escape(&chat.display_name))
        .activatable(true)
        .build();

    let avatar = adw::Avatar::new(40, Some(&chat.display_name), true);
    if let Some(url) = chat.picture_url.as_ref() {
        let proxied = proxied_image_url(url.clone(), prefs.clone(), Some(80), Some(80), true);
        image_cache::fetch_into_avatar(&avatar, &proxied);
    }
    row.add_prefix(&avatar);

    let subtitle = if chat.is_typing {
        "Typing…".to_string()
    } else {
        chat.last_message_preview
            .clone()
            .or_else(|| chat.subtitle.clone())
            .unwrap_or_else(|| "No messages yet".to_string())
    };
    row.set_subtitle(&escape(&subtitle));

    let suffix = gtk::Box::new(gtk::Orientation::Vertical, 4);
    suffix.set_valign(gtk::Align::Center);
    suffix.set_halign(gtk::Align::End);

    if let Some(secs) = chat.last_message_at_secs {
        let label = gtk::Label::new(Some(&relative_time(secs, now)));
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_halign(gtk::Align::End);
        suffix.append(&label);
    }

    if chat.unread_count > 0 {
        let badge = gtk::Label::new(Some(&format!("{}", chat.unread_count)));
        badge.add_css_class("caption");
        badge.add_css_class("accent");
        badge.set_halign(gtk::Align::End);
        suffix.append(&badge);
    }

    if chat.is_muted {
        let muted = gtk::Image::from_icon_name("notifications-disabled-symbolic");
        muted.add_css_class("dim-label");
        muted.set_tooltip_text(Some("muted"));
        row.add_suffix(&muted);
    }
    if chat.is_pinned {
        let pinned = gtk::Image::from_icon_name("view-pin-symbolic");
        pinned.add_css_class("dim-label");
        pinned.set_tooltip_text(Some("pinned"));
        row.add_suffix(&pinned);
    }
    row.add_suffix(&suffix);

    let activate_manager = manager.clone();
    let chat_id = chat.chat_id.clone();
    row.connect_activated(move |_| {
        activate_manager.dispatch(AppAction::OpenChat {
            chat_id: chat_id.clone(),
        });
    });

    attach_context_menu(&row, chat, manager.clone());

    row
}

fn attach_context_menu(row: &adw::ActionRow, chat: &ChatThreadSnapshot, manager: Rc<AppManager>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    let chat = chat.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let popover = chat_context_popover(&chat, &manager);
        popover.set_parent(&widget);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    row.add_controller(gesture);
}

fn chat_context_popover(chat: &ChatThreadSnapshot, manager: &Rc<AppManager>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_autohide(true);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);

    let read_label = if chat.unread_count > 0 {
        "Mark read"
    } else {
        "Mark as unread"
    };
    column.append(&context_button(read_label, {
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        let unread = chat.unread_count == 0;
        move || {
            manager.dispatch(AppAction::SetChatUnread {
                chat_id: chat_id.clone(),
                unread,
            });
        }
    }));

    column.append(&context_button(
        if chat.is_pinned {
            "Unpin chat"
        } else {
            "Pin chat"
        },
        {
            let manager = manager.clone();
            let chat_id = chat.chat_id.clone();
            let pinned = !chat.is_pinned;
            move || {
                manager.dispatch(AppAction::SetChatPinned {
                    chat_id: chat_id.clone(),
                    pinned,
                });
            }
        },
    ));

    column.append(&context_button(
        if chat.is_muted {
            "Unmute chat"
        } else {
            "Mute chat"
        },
        {
            let manager = manager.clone();
            let chat_id = chat.chat_id.clone();
            let muted = !chat.is_muted;
            move || {
                manager.dispatch(AppAction::SetChatMuted {
                    chat_id: chat_id.clone(),
                    muted,
                });
            }
        },
    ));

    column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let delete = context_button("Delete", {
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        move || {
            manager.dispatch(AppAction::DeleteChat {
                chat_id: chat_id.clone(),
            });
        }
    });
    delete.add_css_class("destructive-action");
    column.append(&delete);

    popover.set_child(Some(&column));
    popover
}

fn context_button(label: &str, action: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("flat");
    button.set_halign(gtk::Align::Fill);
    button.connect_clicked(move |button| {
        action();
        if let Some(popover) = button
            .ancestor(gtk::Popover::static_type())
            .and_then(|widget| widget.downcast::<gtk::Popover>().ok())
        {
            popover.popdown();
        }
    });
    button
}

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn nearby_summary(peers: &[iris_chat_core::DesktopNearbyPeerSnapshot]) -> String {
    match peers.len() {
        0 => "Visible".to_string(),
        1 => format!("{} nearby", summary_name(&peers[0].name)),
        2 => format!(
            "{} and {} nearby",
            summary_name(&peers[0].name),
            summary_name(&peers[1].name)
        ),
        count => format!(
            "{}, {} and {} others nearby",
            summary_name(&peers[0].name),
            summary_name(&peers[1].name),
            count - 2
        ),
    }
}

fn summary_name(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Someone"
    } else {
        trimmed
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

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn relative_time(secs: u64, now: u64) -> String {
    if secs == 0 || secs > now {
        return String::new();
    }
    let diff = now - secs;
    if diff < 60 {
        return "now".to_string();
    }
    if diff < 3600 {
        return format!("{}m", diff / 60);
    }
    if diff < 86_400 {
        return format!("{}h", diff / 3600);
    }
    if diff < 86_400 * 7 {
        return format!("{}d", diff / 86_400);
    }
    if diff < 86_400 * 30 {
        return format!("{}w", diff / (86_400 * 7));
    }
    if diff < 86_400 * 365 {
        return format!("{}mo", diff / (86_400 * 30));
    }
    format!("{}y", diff / (86_400 * 365))
}
