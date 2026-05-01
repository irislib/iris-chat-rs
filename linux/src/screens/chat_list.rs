use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use iris_chat_core::{
    proxied_image_url, AppAction, AppState, ChatThreadSnapshot, PreferencesSnapshot,
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

fn nearby_row(manager: &Rc<AppManager>) -> adw::ActionRow {
    let snapshot = manager.nearby_snapshot();
    let subtitle = if !snapshot.visible {
        "Click to enable".to_string()
    } else if !snapshot.peers.is_empty() {
        nearby_summary(&snapshot.peers)
    } else {
        wifi_status_label(&snapshot.status)
    };
    let row = adw::ActionRow::builder()
        .title("Nearby")
        .subtitle(escape(&subtitle))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("network-wireless-symbolic"));
    let manager = manager.clone();
    row.connect_activated(move |row| {
        let parent = row.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        crate::screens::present_nearby(parent.as_ref(), manager.clone());
    });
    row
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
    row.add_suffix(&suffix);

    let manager = manager.clone();
    let chat_id = chat.chat_id.clone();
    row.connect_activated(move |_| {
        manager.dispatch(AppAction::OpenChat {
            chat_id: chat_id.clone(),
        });
    });

    row
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
