use super::*;

pub(super) fn append_rows(parent: &gtk::Box, attachments: &[MessageAttachmentSnapshot]) {
    let mut existing = std::collections::HashMap::new();
    let mut child = parent.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        existing.insert(row.widget_name().to_string(), row);
    }
    let mut previous = None::<gtk::Widget>;
    for (attachment_index, attachment) in attachments.iter().enumerate() {
        let file_key = format!(
            "file:{attachment_index}:{}:{}",
            attachment.htree_url, attachment.filename
        );
        let file = existing.remove(&file_key).unwrap_or_else(|| {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            row.set_widget_name(&file_key);
            let filename = if attachment.filename.is_empty() {
                "File"
            } else {
                &attachment.filename
            };
            info_copy_row(&row, filename, &attachment.htree_url, true);
            parent.append(&row);
            row.upcast()
        });
        parent.reorder_child_after(&file, previous.as_ref());
        previous = Some(file);
        for (index, url) in upload_urls(attachment).enumerate() {
            let key = format!("upload:{attachment_index}:{}:{url}", attachment.htree_url);
            let row = existing.remove(&key).unwrap_or_else(|| {
                let row = upload_row(url);
                row.set_widget_name(&key);
                parent.append(&row);
                row.upcast()
            });
            if let Some(heading) = row.first_child().and_downcast::<gtk::Label>() {
                let text = if index == 0 { "Encrypted uploads" } else { "" };
                if heading.text() != text {
                    heading.set_text(text);
                }
            }
            parent.reorder_child_after(&row, previous.as_ref());
            previous = Some(row);
        }
    }
    for row in existing.into_values() {
        parent.remove(&row);
    }
}

fn upload_row(url: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_start(12);
    row.set_margin_end(12);
    let heading = gtk::Label::new(None);
    heading.add_css_class("dim-label");
    heading.set_xalign(0.0);
    heading.set_width_chars(14);
    row.append(&heading);
    let link = gtk::Label::new(None);
    let escaped = glib::markup_escape_text(url);
    link.set_markup(&format!("<a href=\"{escaped}\">{escaped}</a>"));
    link.set_xalign(0.0);
    link.set_wrap(true);
    link.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    link.set_selectable(true);
    link.set_hexpand(true);
    install_link_actions(&link, vec![url.to_owned()]);
    row.append(&link);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.add_css_class("flat");
    copy.set_tooltip_text(Some("Copy"));
    copy.show_pointer_cursor();
    let url = url.to_owned();
    copy.connect_clicked(move |_| crate::platform::clipboard::copy(&url));
    row.append(&copy);
    row
}

fn upload_urls(attachment: &MessageAttachmentSnapshot) -> impl Iterator<Item = &String> {
    attachment.uploaded_urls.iter().filter(|value| {
        reqwest::Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
    })
}

pub(super) fn text_lines(attachments: &[MessageAttachmentSnapshot]) -> Vec<String> {
    let mut lines = Vec::new();
    for attachment in attachments {
        let filename = if attachment.filename.is_empty() {
            "File"
        } else {
            &attachment.filename
        };
        lines.push(format!("- {filename} {}", attachment.htree_url));
        let uploads = upload_urls(attachment).cloned().collect::<Vec<_>>();
        if !uploads.is_empty() {
            lines.push(format!("  Encrypted uploads: {}", uploads.join(", ")));
        }
    }
    lines
}

pub(super) fn watch_updates(
    dialog: &adw::Dialog,
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
    section: &gtk::Box,
    rows: &gtk::Box,
    info_text: Rc<RefCell<String>>,
) {
    let section = section.downgrade();
    let rows = rows.downgrade();
    let mut shown = message.clone();
    let chat = chat.clone();
    message_actions::watch_message(dialog, message, manager, move |current| {
        if current.deleted_for_everyone != shown.deleted_for_everyone
            || current.body != shown.body
            || current.edit_history != shown.edit_history
        {
            return false;
        }
        if current.attachments != shown.attachments {
            let (Some(section), Some(rows)) = (section.upgrade(), rows.upgrade()) else {
                return false;
            };
            append_rows(&rows, &current.attachments);
            section.set_visible(!current.attachments.is_empty());
            *info_text.borrow_mut() = message_info_text(current, Some(&chat));
            shown = current.clone();
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_links_are_local_details_and_only_open_external_http_blobs() {
        let url = format!("https://files.example/{}.bin", "a".repeat(64));
        let mut attachment = MessageAttachmentSnapshot {
            nhash: "fixture".into(),
            filename: "photo.jpg".into(),
            filename_encoded: "photo.jpg".into(),
            htree_url: "htree://fixture/photo.jpg".into(),
            is_image: true,
            is_video: false,
            is_audio: false,
            uploaded_urls: vec![],
        };
        assert_eq!(
            text_lines(&[attachment.clone()]),
            vec!["- photo.jpg htree://fixture/photo.jpg"]
        );
        attachment.uploaded_urls = vec![
            url.clone(),
            "file:///tmp/photo.jpg".into(),
            "htree://secret/photo.jpg".into(),
            "https://user:secret@files.example/blob".into(),
            "https://files.example/blob#key".into(),
            "https://files.example/blob?secret=key".into(),
        ];
        assert_eq!(
            upload_urls(&attachment).cloned().collect::<Vec<_>>(),
            vec![url.clone()]
        );
        assert!(text_lines(&[attachment.clone()]).join("\n").contains(&url));
        assert_eq!(
            forwardable_attachment_text(&attachment),
            "htree://fixture/photo.jpg"
        );
    }
}
