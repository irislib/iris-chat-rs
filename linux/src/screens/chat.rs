use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState, ChatMessageSnapshot, CurrentChatSnapshot};

use crate::app_manager::AppManager;

pub fn render(chat_id: &str, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_vexpand(true);

    let Some(chat) = state.current_chat.as_ref().filter(|c| c.chat_id == chat_id) else {
        let placeholder = gtk::Label::new(Some("Loading chat…"));
        placeholder.add_css_class("dim-label");
        placeholder.set_vexpand(true);
        container.append(&placeholder);
        return container.upcast();
    };

    container.append(&messages_view(chat));
    container.append(&composer(chat, state, manager));

    container.upcast()
}

fn messages_view(chat: &CurrentChatSnapshot) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    list.set_margin_start(12);
    list.set_margin_end(12);

    if chat.messages.is_empty() {
        let empty = gtk::Label::new(Some("No messages yet"));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        empty.set_valign(gtk::Align::Center);
        list.append(&empty);
    } else {
        for message in &chat.messages {
            list.append(&bubble(message, chat));
        }
    }

    scrolled.set_child(Some(&list));

    let adj = scrolled.vadjustment();
    glib::idle_add_local_once(move || {
        adj.set_value(adj.upper());
    });

    scrolled.upcast()
}

fn bubble(message: &ChatMessageSnapshot, chat: &CurrentChatSnapshot) -> gtk::Widget {
    use ndr_demo_core::{ChatKind, ChatMessageKind};

    if matches!(message.kind, ChatMessageKind::System) {
        let label = gtk::Label::new(Some(&message.body));
        label.add_css_class("dim-label");
        label.add_css_class("caption");
        label.set_halign(gtk::Align::Center);
        label.set_wrap(true);
        return label.upcast();
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_hexpand(true);

    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 2);
    bubble.set_hexpand(false);

    if matches!(chat.kind, ChatKind::Group) && !message.is_outgoing {
        let author = gtk::Label::new(Some(&message.author));
        author.add_css_class("caption");
        author.add_css_class("dim-label");
        author.set_halign(gtk::Align::Start);
        bubble.append(&author);
    }

    let body = gtk::Label::new(Some(&message.body));
    body.set_wrap(true);
    body.set_xalign(0.0);
    body.set_max_width_chars(40);
    body.set_selectable(true);
    bubble.append(&body);

    bubble.add_css_class("card");
    bubble.set_margin_top(2);
    bubble.set_margin_bottom(2);
    bubble.set_margin_start(8);
    bubble.set_margin_end(8);
    bubble.set_size_request(0, -1);

    if message.is_outgoing {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        bubble.add_css_class("accent");
        row.append(&bubble);
    } else {
        row.append(&bubble);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
    }

    row.upcast()
}

fn composer(chat: &CurrentChatSnapshot, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(12);
    row.set_margin_end(12);

    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Message"));
    entry.set_hexpand(true);
    entry.set_height_request(40);
    row.append(&entry);

    let busy = state.busy.sending_message;
    let send = gtk::Button::from_icon_name("document-send-symbolic");
    send.add_css_class("suggested-action");
    send.add_css_class("circular");
    send.set_tooltip_text(Some("Send"));
    send.set_sensitive(!busy);
    row.append(&send);

    let chat_id = chat.chat_id.clone();
    let manager_for_click = manager.clone();
    let entry_for_click = entry.clone();
    send.connect_clicked(move |btn| {
        let text = entry_for_click.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        entry_for_click.set_text("");
        manager_for_click.dispatch(AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text,
        });
    });

    let chat_id = chat.chat_id.clone();
    let manager_for_enter = manager.clone();
    entry.connect_activate(move |entry| {
        let text = entry.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        entry.set_text("");
        manager_for_enter.dispatch(AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text,
        });
    });

    row.upcast()
}
