use super::*;

pub(super) fn nickname_card(info: &ChatInfoSnapshot, manager: Rc<AppManager>) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Nickname and note")
        .build();

    let stored_nickname = info
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let nickname_row = adw::ActionRow::builder()
        .title("Nickname and note")
        .activatable(true)
        .build();
    if let Some(nickname) = stored_nickname.as_deref() {
        nickname_row.set_subtitle(nickname);
    }
    let info_for_edit = info.clone();
    nickname_row.connect_activated(move |row| {
        let parent = row
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_nickname_editor(parent.as_ref(), &info_for_edit, manager.clone());
    });

    group.add(&nickname_row);
    if let Some(note) = info.contact_note.as_deref() {
        let row = adw::ActionRow::builder().title("Note").build();
        row.set_subtitle(&gtk::glib::markup_escape_text(note));
        group.add(&row);
    }

    let primary_name = info
        .nickname
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&info.display_name);
    if let Some(profile_name) = info
        .profile_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(primary_name.trim()))
    {
        let profile_row = adw::ActionRow::builder()
            .title("Profile name")
            .subtitle(profile_name)
            .build();
        group.add(&profile_row);
    }

    group.upcast()
}

fn present_nickname_editor(
    parent: Option<&gtk::Window>,
    info: &ChatInfoSnapshot,
    manager: Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title("Nickname and note")
        .content_width(360)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let nickname_row = adw::EntryRow::builder().title("Nickname").build();
    nickname_row.set_text(info.nickname.as_deref().unwrap_or(""));
    let hint = gtk::Label::new(Some("Only you can see this."));
    hint.add_css_class("dim-label");
    content.append(&hint);
    content.append(&nickname_row);
    let note_label = gtk::Label::new(Some("Note"));
    note_label.set_xalign(0.0);
    content.append(&note_label);
    let note_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let note_buffer = note_view.buffer();
    note_buffer.set_text(info.contact_note.as_deref().unwrap_or(""));
    let scroller = gtk::ScrolledWindow::builder()
        .min_content_height(90)
        .max_content_height(180)
        .child(&note_view)
        .build();
    content.append(&scroller);
    let count = gtk::Label::new(None);
    count.set_xalign(1.0);
    content.append(&count);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let manager_for_save = manager.clone();
    let row_for_save = nickname_row.clone();
    let dialog_for_save = dialog.downgrade();
    let chat_id_for_save = info.chat_id.clone();
    let note_for_save = note_buffer.clone();
    save.connect_clicked(move |_| {
        manager_for_save.dispatch(AppAction::SetContactDetails {
            owner_pubkey_hex: chat_id_for_save.clone(),
            nickname: row_for_save.text().trim().to_string(),
            note: note_for_save
                .text(
                    &note_for_save.start_iter(),
                    &note_for_save.end_iter(),
                    false,
                )
                .to_string(),
        });
        if let Some(dialog) = dialog_for_save.upgrade() {
            dialog.close();
        }
    });
    let validate = {
        let save = save.downgrade();
        let row = nickname_row.downgrade();
        let buffer = note_buffer.downgrade();
        let previous_nickname = info.nickname.clone().unwrap_or_default();
        let previous_note = info.contact_note.clone().unwrap_or_default();
        Rc::new(move || {
            let (Some(save), Some(row), Some(buffer)) =
                (save.upgrade(), row.upgrade(), buffer.upgrade())
            else {
                return;
            };
            let nickname = row.text().split_whitespace().collect::<Vec<_>>().join(" ");
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            let note = normalized.trim();
            let length = note.chars().count();
            save.set_sensitive(
                nickname.chars().count() <= 80
                    && length <= 240
                    && (nickname != previous_nickname || note != previous_note),
            );
            count.set_text(&if nickname.chars().count() > 80 {
                "Use up to 80 characters for a nickname.".to_string()
            } else if length >= 140 {
                format!("{length}/240")
            } else {
                String::new()
            });
        })
    };
    validate();
    let on_nickname = validate.clone();
    nickname_row.connect_changed(move |_| on_nickname());
    note_buffer.connect_changed(move |_| validate());
    actions.append(&save);
    let cancel = gtk::Button::with_label("Cancel");
    let dialog_for_cancel = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(dialog) = dialog_for_cancel.upgrade() {
            dialog.close();
        }
    });
    actions.append(&cancel);

    if info
        .nickname
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || info.contact_note.is_some()
    {
        let remove = gtk::Button::with_label("Remove");
        let manager_for_remove = manager.clone();
        let dialog_for_remove = dialog.downgrade();
        let chat_id_for_remove = info.chat_id.clone();
        remove.connect_clicked(move |_| {
            manager_for_remove.dispatch(AppAction::SetContactDetails {
                owner_pubkey_hex: chat_id_for_remove.clone(),
                nickname: String::new(),
                note: String::new(),
            });
            if let Some(dialog) = dialog_for_remove.upgrade() {
                dialog.close();
            }
        });
        actions.append(&remove);
    }
    content.append(&actions);

    dialog.set_child(Some(&content));
    dialog.present(parent);
}
