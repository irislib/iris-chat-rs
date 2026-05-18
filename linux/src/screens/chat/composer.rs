use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use iris_chat_core::{AppAction, AppState, CurrentChatSnapshot, OutgoingAttachment};

use crate::app_manager::AppManager;
use crate::screens::chat_list::unix_now;

pub(super) fn composer(
    chat: &CurrentChatSnapshot,
    state: &AppState,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
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

    if state.busy.uploading_attachment {
        let progress = gtk::ProgressBar::new();
        progress.set_show_text(false);
        progress.add_css_class("osd");
        if let Some(upload) = state.busy.upload_progress.as_ref() {
            if upload.total_bytes > 0 {
                let fraction =
                    (upload.bytes_uploaded as f64 / upload.total_bytes as f64).clamp(0.0, 1.0);
                progress.set_fraction(fraction);
            } else {
                progress.pulse();
            }
        } else {
            progress.pulse();
        }
        outer.append(&progress);
    }

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
        let parent = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder().title("Attach file").build();
        let manager = manager_for_attach.clone();
        let chat_id = chat_id_for_attach.clone();
        let preview_row = preview_row_for_attach.clone();
        let preview_scroll = preview_scroll_for_attach.clone();
        dialog.open(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
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
            },
        );
    });
    row.append(&attach);

    let buffer = gtk::TextBuffer::new(None);
    let input = gtk::TextView::with_buffer(&buffer);
    input.add_css_class("composer-input");
    input.set_accepts_tab(false);
    input.set_hexpand(true);
    input.set_wrap_mode(gtk::WrapMode::WordChar);
    input.set_top_margin(9);
    input.set_bottom_margin(9);
    input.set_left_margin(12);
    input.set_right_margin(12);

    let input_scroll = gtk::ScrolledWindow::new();
    input_scroll.set_hexpand(true);
    input_scroll.set_min_content_height(40);
    input_scroll.set_max_content_height(132);
    input_scroll.set_propagate_natural_height(true);
    input_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    input_scroll.set_vscrollbar_policy(gtk::PolicyType::Automatic);
    input_scroll.set_child(Some(&input));

    let input_overlay = gtk::Overlay::new();
    input_overlay.set_hexpand(true);
    input_overlay.set_child(Some(&input_scroll));

    let placeholder = gtk::Label::new(Some("Message"));
    placeholder.add_css_class("dim-label");
    placeholder.set_halign(gtk::Align::Start);
    placeholder.set_valign(gtk::Align::Start);
    placeholder.set_margin_start(13);
    placeholder.set_margin_top(10);
    placeholder.set_can_target(false);
    input_overlay.add_overlay(&placeholder);

    // Seed before wiring `connect_changed`; the core dedups identical draft writes.
    if !chat.draft.is_empty() {
        buffer.set_text(&chat.draft);
    }
    placeholder.set_visible(chat.draft.is_empty());
    row.append(&input_overlay);

    let emoji_btn = gtk::Button::from_icon_name("face-smile-symbolic");
    emoji_btn.add_css_class("flat");
    emoji_btn.add_css_class("circular");
    emoji_btn.set_tooltip_text(Some("Insert emoji"));
    let emoji_chooser = gtk::EmojiChooser::new();
    emoji_chooser.set_parent(&emoji_btn);
    {
        let buffer_for_emoji = buffer.clone();
        let input_for_emoji = input.clone();
        emoji_chooser.connect_emoji_picked(move |_, emoji_text| {
            buffer_for_emoji.insert_at_cursor(emoji_text);
            input_for_emoji.grab_focus();
        });
    }
    {
        let chooser_for_click = emoji_chooser.clone();
        emoji_btn.connect_clicked(move |_| chooser_for_click.popup());
    }
    row.append(&emoji_btn);

    if manager.should_focus_composer(&chat.chat_id) {
        let input_for_focus = input.clone();
        gtk::glib::idle_add_local_once(move || {
            input_for_focus.grab_focus();
        });
    }

    {
        let manager_for_typing = manager.clone();
        let chat_id_for_typing = chat.chat_id.clone();
        let placeholder_for_typing = placeholder.clone();
        buffer.connect_changed(move |buffer| {
            let text = composer_buffer_text(buffer);
            placeholder_for_typing.set_visible(text.is_empty());
            if text.is_empty() {
                manager_for_typing.dispatch(AppAction::StopTyping {
                    chat_id: chat_id_for_typing.clone(),
                });
            } else {
                manager_for_typing.dispatch(AppAction::SendTyping {
                    chat_id: chat_id_for_typing.clone(),
                });
            }
            manager_for_typing.dispatch(AppAction::SetChatDraft {
                chat_id: chat_id_for_typing.clone(),
                text,
            });
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
    let buffer_for_click = buffer.clone();
    let preview_row_for_send = preview_row.clone();
    let preview_scroll_for_send = preview_scroll.clone();
    send.connect_clicked(move |btn| {
        if submit_composer(
            &manager_for_click,
            &chat_id,
            &buffer_for_click,
            ttl,
            &preview_row_for_send,
            &preview_scroll_for_send,
        ) {
            btn.set_sensitive(false);
        }
    });

    let chat_id = chat.chat_id.clone();
    let ttl = chat.message_ttl_seconds;
    let manager_for_enter = manager.clone();
    let buffer_for_enter = buffer.clone();
    let preview_row_for_enter = preview_row.clone();
    let preview_scroll_for_enter = preview_scroll.clone();
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    key_controller.connect_key_pressed(move |_, keyval, _, state| {
        if !matches!(keyval, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            || state.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return glib::Propagation::Proceed;
        }

        submit_composer(
            &manager_for_enter,
            &chat_id,
            &buffer_for_enter,
            ttl,
            &preview_row_for_enter,
            &preview_scroll_for_enter,
        );
        glib::Propagation::Stop
    });
    input.add_controller(key_controller);

    outer.append(&row);
    outer.upcast()
}

fn composer_buffer_text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}

fn submit_composer(
    manager: &Rc<AppManager>,
    chat_id: &str,
    buffer: &gtk::TextBuffer,
    ttl_seconds: Option<u64>,
    preview_row: &gtk::Box,
    preview_scroll: &gtk::ScrolledWindow,
) -> bool {
    let text = composer_buffer_text(buffer).trim().to_string();
    let staged = manager.staged_attachments(chat_id);
    if text.is_empty() && staged.is_empty() {
        return false;
    }

    buffer.set_text("");
    dispatch_send(manager, chat_id, text, ttl_seconds);
    rebuild_attachment_previews(preview_row, manager, chat_id);
    preview_scroll.set_visible(preview_row.first_child().is_some());
    true
}

fn rebuild_attachment_previews(row: &gtk::Box, manager: &Rc<AppManager>, chat_id: &str) {
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
    if is_image_filename(&attachment.filename) {
        if let Some(widget) = image_attachment_chip(manager, chat_id, attachment, row) {
            return widget;
        }
    }

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

fn image_attachment_chip(
    manager: &Rc<AppManager>,
    chat_id: &str,
    attachment: &OutgoingAttachment,
    row: &gtk::Box,
) -> Option<gtk::Widget> {
    let texture = gtk::gdk::Texture::from_filename(&attachment.file_path).ok()?;

    let overlay = gtk::Overlay::new();
    overlay.set_margin_top(2);
    overlay.set_margin_bottom(2);
    overlay.set_tooltip_text(Some(&attachment.filename));

    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_can_shrink(true);
    picture.set_size_request(56, 56);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.add_css_class("card");
    overlay.set_child(Some(&picture));

    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.add_css_class("circular");
    remove.add_css_class("osd");
    remove.set_tooltip_text(Some("Remove attachment"));
    remove.set_halign(gtk::Align::End);
    remove.set_valign(gtk::Align::Start);
    remove.set_margin_top(2);
    remove.set_margin_end(2);
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
    overlay.add_overlay(&remove);

    Some(overlay.upcast())
}

fn is_image_filename(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(
        ext,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" | "avif" | "tif" | "tiff"
    )
}

fn attachment_icon_name(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" => "image-x-generic",
        "mp4" | "mov" | "mkv" | "webm" | "avi" => "video-x-generic",
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" => "audio-x-generic",
        "zip" | "tar" | "gz" | "rar" | "7z" => "package-x-generic",
        "pdf" | "doc" | "docx" | "txt" | "md" => "text-x-generic",
        _ => "mail-attachment-symbolic",
    }
}

fn dispatch_send(manager: &Rc<AppManager>, chat_id: &str, text: String, ttl_seconds: Option<u64>) {
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
