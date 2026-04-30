use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use iris_chat_core::{
    proxied_image_url, AppAction, AppState, ChatThreadSnapshot, PreferencesSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::screen_container;
use crate::widgets::image_cache;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    if state.chat_list.is_empty() {
        return empty_state();
    }

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
    for chat in &state.chat_list {
        list.append(&row_for(chat, &state.preferences, now, manager));
    }

    scrolled.set_child(Some(&list));
    scrolled.upcast()
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

fn empty_state() -> gtk::Widget {
    let container = screen_container();
    container.set_valign(gtk::Align::Center);
    container.set_vexpand(true);

    let title = gtk::Label::new(Some("No chats yet"));
    title.add_css_class("title-3");
    container.append(&title);

    let hint = gtk::Label::new(Some("Tap the + in the header to start a new chat."));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    container.append(&hint);

    container.upcast()
}

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
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
