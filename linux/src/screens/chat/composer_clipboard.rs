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
    row: &gtk::Box,
    scroll: &gtk::ScrolledWindow,
) {
    let manager = manager.clone();
    let chat = chat.to_owned();
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
        if !can_attach(&manager, &chat) {
            return;
        }
        let account = manager.current_state().account;
        let manager = manager.clone();
        let chat = chat.clone();
        let input = input.downgrade();
        let row = row.clone();
        let scroll = scroll.clone();
        glib::MainContext::default().spawn_local(async move {
            // File-manager copies also advertise image previews: retain original
            // bytes and every file rather than attaching a duplicate thumbnail.
            let result = if files {
                clipboard
                    .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
                    .await
                    .ok()
                    .and_then(|value| dropped_files(&value))
                    .map(Pasted::Files)
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
                || !can_attach(&manager, &chat)
                || manager.current_state().account != account
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
                None => {
                    // Unsupported/invalid file data may still carry useful text.
                    input
                        .buffer()
                        .paste_clipboard(&clipboard, None, input.is_editable());
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
