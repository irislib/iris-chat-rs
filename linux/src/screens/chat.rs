use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{
    proxied_image_url, AppAction, AppState, ChatKind, ChatMessageKind, ChatMessageSnapshot,
    CurrentChatSnapshot, MessageAttachmentSnapshot, MessageReactionSnapshot, OutgoingAttachment,
    PreferencesSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::chat_list::{relative_time, unix_now};
use crate::widgets::image_cache;

pub struct ChatInfoSnapshot {
    pub chat_id: String,
    pub display_name: String,
    pub subtitle: Option<String>,
    pub picture_url: Option<String>,
    pub preferences: PreferencesSnapshot,
}

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

    mark_visible_seen(chat, manager);

    container.append(&ttl_strip(chat, manager));
    container.append(&messages_view(chat, &state.preferences, manager));
    container.append(&composer(chat, state, manager));

    container.upcast()
}

pub fn present_chat_info(
    parent: Option<&gtk::Window>,
    info: ChatInfoSnapshot,
    manager: Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title(&info.display_name)
        .content_width(360)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    header_row.set_halign(gtk::Align::Start);

    let avatar = adw::Avatar::new(72, Some(&info.display_name), true);
    if let Some(url) = info.picture_url.as_deref() {
        if url.starts_with("http://") || url.starts_with("https://") {
            let proxied = proxied_image_url(
                url.to_string(),
                info.preferences.clone(),
                Some(144),
                Some(144),
                true,
            );
            image_cache::fetch_into_avatar(&avatar, &proxied);
        }
    }
    header_row.append(&avatar);

    let text_column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text_column.set_valign(gtk::Align::Center);

    let header = gtk::Label::new(Some(&info.display_name));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    header.set_xalign(0.0);
    header.set_wrap(true);
    text_column.append(&header);

    if let Some(subtitle) = info.subtitle.as_deref().filter(|s| !s.is_empty()) {
        let sub = gtk::Label::new(Some(subtitle));
        sub.add_css_class("dim-label");
        sub.set_wrap(true);
        sub.set_max_width_chars(36);
        sub.set_xalign(0.0);
        sub.set_halign(gtk::Align::Start);
        text_column.append(&sub);
    }
    header_row.append(&text_column);
    content.append(&header_row);

    let delete = gtk::Button::with_label("Delete chat");
    delete.add_css_class("destructive-action");
    delete.set_halign(gtk::Align::Start);
    let manager_for_delete = manager.clone();
    let chat_id_for_delete = info.chat_id.clone();
    let dialog_for_delete = dialog.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteChat {
            chat_id: chat_id_for_delete.clone(),
        });
        dialog_for_delete.close();
    });
    content.append(&delete);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn ttl_strip(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row.set_margin_top(6);
    row.set_halign(gtk::Align::End);

    let label = match chat.message_ttl_seconds {
        None | Some(0) => "No expiry".to_string(),
        Some(s) if s < 3600 => format!("Expires {}m", s / 60),
        Some(s) if s < 86_400 => format!("Expires {}h", s / 3600),
        Some(s) if s < 86_400 * 7 => format!("Expires {}d", s / 86_400),
        Some(s) => format!("Expires {}w", s / (86_400 * 7)),
    };

    let menu_button = gtk::MenuButton::new();
    menu_button.set_label(&label);
    menu_button.add_css_class("flat");
    menu_button.add_css_class("caption");

    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);

    let options: &[(&str, Option<u64>)] = &[
        ("No expiry", None),
        ("1 hour", Some(3600)),
        ("6 hours", Some(6 * 3600)),
        ("1 day", Some(86_400)),
        ("1 week", Some(7 * 86_400)),
    ];
    for (option_label, ttl) in options {
        let item = gtk::Button::with_label(option_label);
        item.add_css_class("flat");
        item.set_halign(gtk::Align::Fill);
        let manager = manager.clone();
        let chat_id = chat.chat_id.clone();
        let ttl_value = *ttl;
        let popover_for_close = popover.clone();
        item.connect_clicked(move |_| {
            manager.dispatch(AppAction::SetChatMessageTtl {
                chat_id: chat_id.clone(),
                ttl_seconds: ttl_value,
            });
            popover_for_close.popdown();
        });
        list.append(&item);
    }
    popover.set_child(Some(&list));
    menu_button.set_popover(Some(&popover));

    row.append(&menu_button);
    row.upcast()
}

fn messages_view(
    chat: &CurrentChatSnapshot,
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_vexpand(true);
    list.set_valign(gtk::Align::End);
    list.set_margin_top(8);
    list.set_margin_bottom(8);
    list.set_margin_start(10);
    list.set_margin_end(10);

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
                prefs,
                manager,
            ));

            last_author = Some(message.author.clone());
            last_outgoing = message.is_outgoing;
            last_secs = message.created_at_secs;
        }
    }

    scrolled.set_child(Some(&list));

    let adj = scrolled.vadjustment();
    adj.connect_changed(|adj| {
        let bottom = (adj.upper() - adj.page_size()).max(adj.lower());
        adj.set_value(bottom);
    });
    glib::idle_add_local_once(move || {
        let bottom = (adj.upper() - adj.page_size()).max(adj.lower());
        adj.set_value(bottom);
    });

    scrolled.upcast()
}

fn mark_visible_seen(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) {
    let unseen: Vec<String> = chat
        .messages
        .iter()
        .filter(|m| !m.is_outgoing && matches!(m.kind, ChatMessageKind::User))
        .map(|m| m.id.clone())
        .collect();
    if unseen.is_empty() {
        return;
    }
    manager.dispatch(AppAction::MarkMessagesSeen {
        chat_id: chat.chat_id.clone(),
        message_ids: unseen,
    });
}

fn render_message(
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    cluster_start: bool,
    cluster_end: bool,
    _is_last: bool,
    now: u64,
    prefs: &PreferencesSnapshot,
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
    row.set_margin_top(if cluster_start { 8 } else { 2 });
    row.set_margin_bottom(if cluster_end { 4 } else { 0 });

    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_hexpand(false);

    if matches!(chat.kind, ChatKind::Group) && !message.is_outgoing && cluster_start {
        let author = gtk::Label::new(Some(&message.author));
        author.add_css_class("chat-author");
        author.set_halign(gtk::Align::Start);
        column.append(&author);
    }

    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 2);
    bubble.add_css_class(if message.is_outgoing {
        "bubble-out"
    } else {
        "bubble-in"
    });
    bubble.set_halign(if message.is_outgoing {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });

    let image_attachments: Vec<&MessageAttachmentSnapshot> =
        message.attachments.iter().filter(|a| a.is_image).collect();
    let other_attachments: Vec<&MessageAttachmentSnapshot> =
        message.attachments.iter().filter(|a| !a.is_image).collect();

    for attachment in &image_attachments {
        bubble.append(&image_bubble(attachment, prefs));
    }

    if !message.body.is_empty() {
        let body = gtk::Label::new(Some(&message.body));
        body.set_wrap(true);
        body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        body.set_xalign(0.0);
        body.set_max_width_chars(40);
        body.set_selectable(true);
        bubble.append(&body);
    }

    if !other_attachments.is_empty() {
        let attach_summary = gtk::Label::new(Some(&attachment_summary(&other_attachments)));
        attach_summary.add_css_class("bubble-meta");
        attach_summary.set_xalign(0.0);
        bubble.append(&attach_summary);
    }

    if cluster_end {
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        footer.add_css_class("bubble-meta");
        let time = gtk::Label::new(Some(&relative_time(message.created_at_secs, now)));
        footer.append(&time);
        if message.is_outgoing {
            let glyph = gtk::Label::new(Some(delivery_glyph(&message.delivery)));
            footer.append(&glyph);
        }
        footer.set_halign(gtk::Align::End);
        footer.set_margin_top(2);
        bubble.append(&footer);
    }

    let popover = build_message_popover(message, manager);
    popover.set_parent(&bubble);
    let popover_for_gesture = popover.clone();
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.connect_pressed(move |_, _, x, y| {
        popover_for_gesture.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_gesture.popup();
    });
    bubble.add_controller(gesture);

    let popover_for_long = popover.clone();
    let long_press = gtk::GestureLongPress::new();
    long_press.connect_pressed(move |_, x, y| {
        popover_for_long.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_long.popup();
    });
    bubble.add_controller(long_press);

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

fn build_message_popover(
    message: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);

    let reactions_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for emoji in ["👍", "❤️", "😂", "🎉", "😢", "🔥"] {
        let btn = gtk::Button::with_label(emoji);
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        let manager = manager.clone();
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let emoji_owned = emoji.to_string();
        let popover_for_close = popover.clone();
        btn.connect_clicked(move |_| {
            manager.dispatch(AppAction::ToggleReaction {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                emoji: emoji_owned.clone(),
            });
            popover_for_close.popdown();
        });
        reactions_row.append(&btn);
    }
    column.append(&reactions_row);

    let copy = gtk::Button::with_label("Copy text");
    copy.add_css_class("flat");
    copy.set_halign(gtk::Align::Fill);
    let body = message.body.clone();
    let popover_for_copy = popover.clone();
    copy.connect_clicked(move |_| {
        crate::platform::clipboard::copy(&body);
        popover_for_copy.popdown();
    });
    column.append(&copy);

    let delete = gtk::Button::with_label("Delete locally");
    delete.add_css_class("flat");
    delete.add_css_class("error");
    delete.set_halign(gtk::Align::Fill);
    let manager_for_delete = manager.clone();
    let chat_id = message.chat_id.clone();
    let message_id = message.id.clone();
    let popover_for_delete = popover.clone();
    delete.connect_clicked(move |_| {
        manager_for_delete.dispatch(AppAction::DeleteLocalMessage {
            chat_id: chat_id.clone(),
            message_id: message_id.clone(),
        });
        popover_for_delete.popdown();
    });
    column.append(&delete);

    popover.set_child(Some(&column));
    popover
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
    chip.add_css_class("chat-day");
    chip.set_halign(gtk::Align::Center);
    chip.set_margin_top(12);
    chip.set_margin_bottom(6);
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

fn delivery_glyph(state: &iris_chat_core::DeliveryState) -> &'static str {
    use iris_chat_core::DeliveryState::*;
    match state {
        Queued => "⋯",
        Pending => "⋯",
        Sent => "✓",
        Received => "✓✓",
        Seen => "✓✓",
        Failed => "!",
    }
}

fn attachment_summary(attachments: &[&MessageAttachmentSnapshot]) -> String {
    if attachments.len() == 1 {
        let a = attachments[0];
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

fn image_bubble(attachment: &MessageAttachmentSnapshot, prefs: &PreferencesSnapshot) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_size_request(220, 220);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.add_css_class("card");

    let url = proxied_image_url(
        attachment.htree_url.clone(),
        prefs.clone(),
        Some(440),
        Some(440),
        false,
    );
    image_cache::fetch_into_picture(&picture, &url);
    picture.upcast()
}

fn composer(chat: &CurrentChatSnapshot, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.set_margin_top(8);
    outer.set_margin_bottom(8);
    outer.set_margin_start(12);
    outer.set_margin_end(12);

    let preview_scroll = gtk::ScrolledWindow::new();
    preview_scroll.set_hscrollbar_policy(gtk::PolicyType::Automatic);
    preview_scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
    preview_scroll.set_propagate_natural_height(true);
    let preview_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    preview_scroll.set_child(Some(&preview_row));
    outer.append(&preview_scroll);
    rebuild_attachment_previews(&preview_row, manager, &chat.chat_id);
    preview_scroll.set_visible(preview_row.first_child().is_some());

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    let attach = gtk::Button::from_icon_name("mail-attachment-symbolic");
    attach.add_css_class("flat");
    attach.add_css_class("circular");
    attach.set_tooltip_text(Some("Attach file"));
    attach.set_sensitive(!state.busy.uploading_attachment);
    let manager_for_attach = manager.clone();
    let chat_id_for_attach = chat.chat_id.clone();
    let preview_row_for_attach = preview_row.clone();
    let preview_scroll_for_attach = preview_scroll.clone();
    attach.connect_clicked(move |btn| {
        let parent = btn
            .root()
            .and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder().title("Attach file").build();
        let manager = manager_for_attach.clone();
        let chat_id = chat_id_for_attach.clone();
        let preview_row = preview_row_for_attach.clone();
        let preview_scroll = preview_scroll_for_attach.clone();
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
            manager.stage_attachment(
                &chat_id,
                OutgoingAttachment {
                    file_path: path,
                    filename,
                },
            );
            rebuild_attachment_previews(&preview_row, &manager, &chat_id);
            preview_scroll.set_visible(preview_row.first_child().is_some());
        });
    });
    row.append(&attach);

    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Message"));
    entry.set_hexpand(true);
    entry.set_height_request(40);
    row.append(&entry);

    if manager.should_focus_composer(&chat.chat_id) {
        let entry_for_focus = entry.clone();
        gtk::glib::idle_add_local_once(move || {
            entry_for_focus.grab_focus();
        });
    }

    {
        let manager_for_typing = manager.clone();
        let chat_id_for_typing = chat.chat_id.clone();
        entry.connect_changed(move |e| {
            if e.text().is_empty() {
                manager_for_typing.dispatch(AppAction::StopTyping {
                    chat_id: chat_id_for_typing.clone(),
                });
            } else {
                manager_for_typing.dispatch(AppAction::SendTyping {
                    chat_id: chat_id_for_typing.clone(),
                });
            }
        });
    }

    let busy = state.busy.sending_message;
    let send = gtk::Button::from_icon_name("document-send-symbolic");
    send.add_css_class("suggested-action");
    send.add_css_class("circular");
    send.set_tooltip_text(Some("Send"));
    send.set_sensitive(!busy);
    row.append(&send);

    let chat_id = chat.chat_id.clone();
    let ttl = chat.message_ttl_seconds;
    let manager_for_click = manager.clone();
    let entry_for_click = entry.clone();
    let preview_row_for_send = preview_row.clone();
    let preview_scroll_for_send = preview_scroll.clone();
    send.connect_clicked(move |btn| {
        let text = entry_for_click.text().trim().to_string();
        let staged = manager_for_click.staged_attachments(&chat_id);
        if text.is_empty() && staged.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        entry_for_click.set_text("");
        dispatch_send(&manager_for_click, &chat_id, text, ttl);
        rebuild_attachment_previews(&preview_row_for_send, &manager_for_click, &chat_id);
        preview_scroll_for_send.set_visible(preview_row_for_send.first_child().is_some());
    });

    let chat_id = chat.chat_id.clone();
    let ttl = chat.message_ttl_seconds;
    let manager_for_enter = manager.clone();
    let preview_row_for_enter = preview_row.clone();
    let preview_scroll_for_enter = preview_scroll.clone();
    entry.connect_activate(move |entry| {
        let text = entry.text().trim().to_string();
        let staged = manager_for_enter.staged_attachments(&chat_id);
        if text.is_empty() && staged.is_empty() {
            return;
        }
        entry.set_text("");
        dispatch_send(&manager_for_enter, &chat_id, text, ttl);
        rebuild_attachment_previews(&preview_row_for_enter, &manager_for_enter, &chat_id);
        preview_scroll_for_enter.set_visible(preview_row_for_enter.first_child().is_some());
    });

    outer.append(&row);
    outer.upcast()
}

fn rebuild_attachment_previews(
    row: &gtk::Box,
    manager: &Rc<AppManager>,
    chat_id: &str,
) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    for attachment in manager.staged_attachments(chat_id) {
        row.append(&attachment_chip(manager, chat_id, &attachment, row));
    }
}

fn attachment_chip(
    manager: &Rc<AppManager>,
    chat_id: &str,
    attachment: &OutgoingAttachment,
    row: &gtk::Box,
) -> gtk::Widget {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chip.add_css_class("card");
    chip.set_margin_top(2);
    chip.set_margin_bottom(2);

    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    inner.set_margin_top(6);
    inner.set_margin_bottom(6);
    inner.set_margin_start(10);
    inner.set_margin_end(4);

    let icon = gtk::Image::from_icon_name(attachment_icon_name(&attachment.filename));
    icon.set_pixel_size(20);
    inner.append(&icon);

    let label = gtk::Label::new(Some(&attachment.filename));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(24);
    label.set_xalign(0.0);
    inner.append(&label);

    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.add_css_class("flat");
    remove.add_css_class("circular");
    remove.set_tooltip_text(Some("Remove attachment"));
    let manager_for_remove = manager.clone();
    let chat_id_for_remove = chat_id.to_string();
    let file_path = attachment.file_path.clone();
    let row_for_remove = row.clone();
    remove.connect_clicked(move |_| {
        manager_for_remove.unstage_attachment(&chat_id_for_remove, &file_path);
        rebuild_attachment_previews(&row_for_remove, &manager_for_remove, &chat_id_for_remove);
        if let Some(scroll) = row_for_remove
            .parent()
            .and_then(|p| p.downcast::<gtk::ScrolledWindow>().ok())
        {
            scroll.set_visible(row_for_remove.first_child().is_some());
        }
    });
    inner.append(&remove);

    chip.append(&inner);
    chip.upcast()
}

fn attachment_icon_name(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    let ext = lower
        .rsplit_once('.')
        .map(|(_, e)| e)
        .unwrap_or("");
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" => "image-x-generic",
        "mp4" | "mov" | "mkv" | "webm" | "avi" => "video-x-generic",
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" => "audio-x-generic",
        "zip" | "tar" | "gz" | "rar" | "7z" => "package-x-generic",
        "pdf" | "doc" | "docx" | "txt" | "md" => "text-x-generic",
        _ => "mail-attachment-symbolic",
    }
}

fn dispatch_send(
    manager: &Rc<AppManager>,
    chat_id: &str,
    text: String,
    ttl_seconds: Option<u64>,
) {
    let staged = manager.take_staged_attachments(chat_id);
    if !staged.is_empty() {
        manager.dispatch(AppAction::SendAttachments {
            chat_id: chat_id.to_string(),
            attachments: staged,
            caption: text,
        });
        return;
    }
    manager.dispatch(send_action(chat_id, text, ttl_seconds));
}

fn send_action(chat_id: &str, text: String, ttl_seconds: Option<u64>) -> AppAction {
    if let Some(ttl) = ttl_seconds.filter(|t| *t > 0) {
        let now = unix_now();
        AppAction::SendDisappearingMessage {
            chat_id: chat_id.to_string(),
            text,
            expires_at_secs: now + ttl,
        }
    } else {
        AppAction::SendMessage {
            chat_id: chat_id.to_string(),
            text,
        }
    }
}
