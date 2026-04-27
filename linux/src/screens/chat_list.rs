use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState, ChatThreadSnapshot};

use crate::app_manager::AppManager;
use crate::screens::screen_container;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    if state.chat_list.is_empty() {
        return empty_state().upcast();
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

    for chat in &state.chat_list {
        list.append(&row_for(chat, manager));
    }

    scrolled.set_child(Some(&list));
    scrolled.upcast()
}

fn row_for(chat: &ChatThreadSnapshot, manager: &Rc<AppManager>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(escape(&chat.display_name))
        .activatable(true)
        .build();

    let subtitle = if chat.is_typing {
        "Typing…".to_string()
    } else {
        chat.last_message_preview
            .clone()
            .or_else(|| chat.subtitle.clone())
            .unwrap_or_else(|| "No messages yet".to_string())
    };
    row.set_subtitle(&escape(&subtitle));

    if chat.unread_count > 0 {
        let badge = gtk::Label::new(Some(&format!("{}", chat.unread_count)));
        badge.add_css_class("caption");
        badge.add_css_class("accent");
        row.add_suffix(&badge);
    }

    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("dim-label");
    row.add_suffix(&chevron);

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
