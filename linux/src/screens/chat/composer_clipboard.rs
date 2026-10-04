use std::io::Write;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::composer::{can_attach, dropped_files, rebuild_attachment_previews};
use crate::app_manager::AppManager;

#[cfg(feature = "ui-tests")]
pub mod tests;

pub(super) fn install(
    input: &gtk::TextView,
    manager: &Rc<AppManager>,
    chat: &str,
    direct: &gtk::CheckButton,
    row: &gtk::Box,
    scroll: &gtk::ScrolledWindow,
) {
    let manager = manager.clone();
    let chat = chat.to_owned();
    let direct = direct.downgrade();
    let row = row.downgrade();
    let scroll = scroll.downgrade();
    input.connect_paste_clipboard(move |input| {
        let clipboard = input.clipboard();
        let formats = clipboard.formats();
        let files = formats.contains_type(gdk::FileList::static_type())
            || formats.contain_mime_type("text/uri-list");
        let image = formats.contains_type(gdk::Texture::static_type())
            || [
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/tiff",
                "image/bmp",
                "image/webp",
            ]
            .iter()
            .any(|mime| formats.contain_mime_type(mime));
        if !files && !image {
            return;
        } // Ordinary text uses GTK's native paste.
        input.stop_signal_emission_by_name("paste-clipboard");
        if input.has_css_class("editing-message") || !can_attach(&manager, &chat) {
            return;
        }
        let account = manager.current_state().account;
        let generation = manager.attachment_draft_generation();
        let Some(direct) = direct.upgrade() else {
            return;
        };
        let send_directly = direct.is_active();
        let manager = manager.clone();
        let chat = chat.clone();
        let input = input.downgrade();
        let row = row.clone();
        let scroll = scroll.clone();
        glib::MainContext::default().spawn_local(async move {
            // File-manager copies also advertise image previews: retain original
            // bytes and every file rather than attaching a duplicate thumbnail.
            let result = if files {
                let files = clipboard
                    .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
                    .await
                    .ok()
                    .and_then(|value| pasted_files(&value));
                if matches!(files.as_ref(), Some(Pasted::UrlText)) && image {
                    // A browser's Copy Image may include its source URL too.
                    clipboard
                        .read_texture_future()
                        .await
                        .ok()
                        .flatten()
                        .map(Pasted::Image)
                } else {
                    files
                }
            } else {
                clipboard
                    .read_texture_future()
                    .await
                    .ok()
                    .flatten()
                    .map(Pasted::Image)
            };
            let (Some(input), Some(row), Some(scroll)) =
                (input.upgrade(), row.upgrade(), scroll.upgrade())
            else {
                return;
            };
            if input.root().is_none()
                || input.has_css_class("editing-message")
                || !can_attach(&manager, &chat)
                || manager.current_state().account != account
                || manager.attachment_draft_generation() != generation
                || direct.is_active() != send_directly
            {
                return;
            }
            match result {
                Some(Pasted::Files(files)) => {
                    for file in files {
                        manager.stage_attachment(&chat, file);
                    }
                }
                Some(Pasted::Image(texture)) => match png_file(&texture) {
                    Ok(file) => manager.stage_clipboard_image(&chat, file),
                    Err(_) => {
                        manager.show_toast("Could not paste image.");
                        return;
                    }
                },
                Some(Pasted::UrlText) => {
                    // Browsers may advertise an ordinary copied URL as both
                    // text and a URI list. It remains a normal text paste.
                    input
                        .buffer()
                        .paste_clipboard(&clipboard, None, input.is_editable());
                    return;
                }
                None => {
                    // A failed file selection may carry file-manager labels or
                    // URLs too. Never put those into the caption as a fallback.
                    manager.show_toast("Could not paste files.");
                    return;
                }
            }
            rebuild_attachment_previews(&row, &manager, &chat);
            scroll.set_visible(row.first_child().is_some());
        });
    });
}

enum Pasted {
    Files(Vec<iris_chat_core::OutgoingAttachment>),
    Image(gdk::Texture),
    UrlText,
}

fn pasted_files(value: &glib::Value) -> Option<Pasted> {
    let list = value.get::<gdk::FileList>().ok()?;
    let files = list.files();
    if !files.is_empty()
        && files.iter().all(|file| {
            let uri = file.uri();
            uri.starts_with("https://") || uri.starts_with("http://")
        })
    {
        return Some(Pasted::UrlText);
    }
    dropped_files(value).map(Pasted::Files)
}

fn png_file(texture: &gdk::Texture) -> std::io::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("iris-clipboard-")
        .suffix(".png")
        .tempfile()?;
    file.write_all(texture.save_to_png_bytes().as_ref())?;
    file.flush()?;
    Ok(file)
}
