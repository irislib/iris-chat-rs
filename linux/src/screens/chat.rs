use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{
    AppAction, AppState, ChatKind, ChatMessageKind, ChatMessageSnapshot, CurrentChatSnapshot,
    MessageReactionSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::chat_list::{relative_time, unix_now};

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

    container.append(&messages_view(chat, manager));
    container.append(&composer(chat, state, manager));

    container.upcast()
}

fn messages_view(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
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
        let mut last_day: Option<String> = None;
        let mut last_author: Option<String> = None;
        let mut last_outgoing = false;
        let mut last_secs: u64 = 0;

        let now = unix_now();

        for (idx, message) in chat.messages.iter().enumerate() {
            let day = day_label_secs(message.created_at_secs);
            let same_day = matches!(last_day.as_deref(), Some(d) if d == day);
            if !same_day {
                list.append(&day_chip(&day));
                last_author = None;
            }
            last_day = Some(day);

            let cluster_break = !matches!(&last_author, Some(a) if a == &message.author)
                || last_outgoing != message.is_outgoing
                || message.created_at_secs.saturating_sub(last_secs) > 300;

            let is_last = idx + 1 == chat.messages.len();
            let next_message = chat.messages.get(idx + 1);
            let cluster_ends = match next_message {
                Some(next) => {
                    next.author != message.author
                        || next.is_outgoing != message.is_outgoing
                        || next.created_at_secs.saturating_sub(message.created_at_secs) > 300
                        || day_label_secs(next.created_at_secs)
                            != day_label_secs(message.created_at_secs)
                }
                None => true,
            };

            list.append(&render_message(
                message,
                chat,
                cluster_break,
                cluster_ends,
                is_last,
                now,
                manager,
            ));

            last_author = Some(message.author.clone());
            last_outgoing = message.is_outgoing;
            last_secs = message.created_at_secs;
        }
    }

    scrolled.set_child(Some(&list));

    let adj = scrolled.vadjustment();
    glib::idle_add_local_once(move || {
        adj.set_value(adj.upper());
    });

    scrolled.upcast()
}

fn render_message(
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    cluster_start: bool,
    cluster_end: bool,
    _is_last: bool,
    now: u64,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    if matches!(message.kind, ChatMessageKind::System) {
        let label = gtk::Label::new(Some(&message.body));
        label.add_css_class("dim-label");
        label.add_css_class("caption");
        label.set_halign(gtk::Align::Center);
        label.set_wrap(true);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        return label.upcast();
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_hexpand(true);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_hexpand(false);

    if matches!(chat.kind, ChatKind::Group) && !message.is_outgoing && cluster_start {
        let author = gtk::Label::new(Some(&message.author));
        author.add_css_class("caption");
        author.add_css_class("dim-label");
        author.set_halign(gtk::Align::Start);
        author.set_margin_start(8);
        column.append(&author);
    }

    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 4);
    bubble.add_css_class("card");
    bubble.set_margin_start(8);
    bubble.set_margin_end(8);

    let body_text = if message.body.is_empty() && !message.attachments.is_empty() {
        attachment_summary(&message.attachments)
    } else {
        message.body.clone()
    };
    let body = gtk::Label::new(Some(&body_text));
    body.set_wrap(true);
    body.set_xalign(0.0);
    body.set_max_width_chars(40);
    body.set_selectable(true);
    bubble.append(&body);

    if !message.attachments.is_empty() && !message.body.is_empty() {
        let attach_summary = gtk::Label::new(Some(&attachment_summary(&message.attachments)));
        attach_summary.add_css_class("caption");
        attach_summary.add_css_class("dim-label");
        attach_summary.set_xalign(0.0);
        bubble.append(&attach_summary);
    }

    if cluster_end {
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let time = gtk::Label::new(Some(&relative_time(message.created_at_secs, now)));
        time.add_css_class("caption");
        time.add_css_class("dim-label");
        footer.append(&time);
        if message.is_outgoing {
            let glyph = gtk::Label::new(Some(delivery_glyph(&message.delivery)));
            glyph.add_css_class("caption");
            glyph.add_css_class("dim-label");
            footer.append(&glyph);
        }
        footer.set_halign(if message.is_outgoing {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        bubble.append(&footer);
    }

    if message.is_outgoing {
        bubble.add_css_class("accent");
    }

    column.append(&bubble);

    if !message.reactions.is_empty() {
        column.append(&reactions_row(message, &message.reactions, manager));
    }

    if message.is_outgoing {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        row.append(&column);
    } else {
        row.append(&column);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
    }

    row.upcast()
}

fn reactions_row(
    message: &ChatMessageSnapshot,
    reactions: &[MessageReactionSnapshot],
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.set_margin_start(8);
    row.set_margin_end(8);
    row.set_halign(if message.is_outgoing {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    for reaction in reactions {
        let chip = gtk::Button::with_label(&format!("{} {}", reaction.emoji, reaction.count));
        chip.add_css_class("pill");
        chip.add_css_class("flat");
        if reaction.reacted_by_me {
            chip.add_css_class("suggested-action");
        }
        let manager = manager.clone();
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let emoji = reaction.emoji.clone();
        chip.connect_clicked(move |_| {
            manager.dispatch(AppAction::ToggleReaction {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                emoji: emoji.clone(),
            });
        });
        row.append(&chip);
    }
    row.upcast()
}

fn day_chip(label: &str) -> gtk::Widget {
    let chip = gtk::Label::new(Some(label));
    chip.add_css_class("caption");
    chip.add_css_class("dim-label");
    chip.set_halign(gtk::Align::Center);
    chip.set_margin_top(12);
    chip.set_margin_bottom(4);
    chip.upcast()
}

fn day_label_secs(secs: u64) -> String {
    let now = unix_now();
    if secs == 0 || secs > now {
        return "—".to_string();
    }
    let now_day = now / 86_400;
    let secs_day = secs / 86_400;
    let diff = now_day.saturating_sub(secs_day);
    match diff {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => format!("{} days ago", diff),
        _ => format!("{}d ago", diff),
    }
}

fn delivery_glyph(state: &ndr_demo_core::DeliveryState) -> &'static str {
    use ndr_demo_core::DeliveryState::*;
    match state {
        Queued => "⋯",
        Pending => "⋯",
        Sent => "✓",
        Received => "✓✓",
        Seen => "✓✓",
        Failed => "!",
    }
}

fn attachment_summary(attachments: &[ndr_demo_core::MessageAttachmentSnapshot]) -> String {
    if attachments.len() == 1 {
        let a = &attachments[0];
        if a.is_image {
            return format!("📷 {}", a.filename);
        }
        if a.is_video {
            return format!("🎞 {}", a.filename);
        }
        if a.is_audio {
            return format!("🔊 {}", a.filename);
        }
        return format!("📎 {}", a.filename);
    }
    format!("📎 {} attachments", attachments.len())
}

fn composer(chat: &CurrentChatSnapshot, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(12);
    row.set_margin_end(12);

    let attach = gtk::Button::from_icon_name("mail-attachment-symbolic");
    attach.add_css_class("flat");
    attach.add_css_class("circular");
    attach.set_tooltip_text(Some("Attach file"));
    attach.set_sensitive(!state.busy.uploading_attachment);
    let manager_for_attach = manager.clone();
    let chat_id_for_attach = chat.chat_id.clone();
    attach.connect_clicked(move |btn| {
        let parent = btn
            .root()
            .and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder().title("Attach file").build();
        let manager = manager_for_attach.clone();
        let chat_id = chat_id_for_attach.clone();
        dialog.open(parent.as_ref(), gtk::gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let path = match file.path() {
                Some(p) => p.to_string_lossy().to_string(),
                None => return,
            };
            let filename = file
                .basename()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "attachment".to_string());
            manager.dispatch(AppAction::SendAttachment {
                chat_id: chat_id.clone(),
                file_path: path,
                filename,
                caption: String::new(),
            });
        });
    });
    row.append(&attach);

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
