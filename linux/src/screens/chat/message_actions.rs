use super::*;

pub(super) fn can_delete_for_everyone(message: &ChatMessageSnapshot) -> bool {
    message.is_outgoing
        && message.kind == ChatMessageKind::User
        && !message.deleted_for_everyone
        && message.call.is_none()
        && message.direct_transfer.is_none()
        && matches!(
            message.delivery,
            DeliveryState::Sent | DeliveryState::Received | DeliveryState::Seen
        )
}

pub(super) fn can_edit(message: &ChatMessageSnapshot) -> bool {
    can_delete_for_everyone(message)
        && !message.body.trim().is_empty()
        && message.attachments.is_empty()
}

pub(super) fn append_actions(
    column: &gtk::Box,
    popover: &gtk::Popover,
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
) {
    if !is_removed_group(chat) && can_edit(message) {
        let edit = gtk::Button::with_label("Edit message");
        edit.add_css_class("flat");
        let id = message.id.clone();
        let popover = popover.clone();
        edit.connect_clicked(move |button| {
            let _ = button.activate_action("message.edit", Some(&id.to_variant()));
            popover.popdown();
        });
        column.append(&edit);
    }
    if !message.deleted_for_everyone && !message.edit_history.is_empty() {
        let history = gtk::Button::with_label("Edit history");
        history.add_css_class("flat");
        let target = message.clone();
        let manager = manager.clone();
        let popover = popover.clone();
        history.connect_clicked(move |button| {
            let parent = button
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            present_history(parent.as_ref(), &target, &manager);
            popover.popdown();
        });
        column.append(&history);
    }
    if !is_removed_group(chat) && can_delete_for_everyone(message) {
        let delete = gtk::Button::with_label("Delete for everyone");
        delete.add_css_class("flat");
        delete.add_css_class("error");
        let target = message.clone();
        let manager = manager.clone();
        let popover = popover.clone();
        delete.connect_clicked(move |button| {
            let parent = button
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            confirm_delete(parent.as_ref(), &target, &manager);
            popover.popdown();
        });
        column.append(&delete);
    }
}

fn confirm_delete(
    parent: Option<&gtk::Window>,
    message: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title("Delete for everyone")
        .content_width(360)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);
    let label = gtk::Label::new(Some(
        "Ask everyone to delete this message and its edit history?",
    ));
    label.set_wrap(true);
    content.append(&label);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    actions.append(&cancel);
    let delete = gtk::Button::with_label("Delete for everyone");
    delete.add_css_class("destructive-action");
    let weak = dialog.downgrade();
    let manager = manager.clone();
    let chat_id = message.chat_id.clone();
    let message_id = message.id.clone();
    delete.connect_clicked(move |_| {
        manager.dispatch(AppAction::DeleteMessageForEveryone {
            chat_id: chat_id.clone(),
            message_id: message_id.clone(),
        });
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    actions.append(&delete);
    content.append(&actions);
    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&cancel));
    dialog.present(parent);
}

pub(super) fn history_button(
    message: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Button {
    let button = gtk::Button::with_label("Edited");
    button.add_css_class("flat");
    button.add_css_class("caption");
    button.set_tooltip_text(Some("Edit history"));
    let target = message.clone();
    let manager = manager.clone();
    button.connect_clicked(move |button| {
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_history(parent.as_ref(), &target, &manager);
    });
    button
}

fn present_history(
    parent: Option<&gtk::Window>,
    target: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
) {
    let dialog = adw::Dialog::builder()
        .title("Edit history")
        .content_width(440)
        .content_height(500)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let versions = gtk::Box::new(gtk::Orientation::Vertical, 16);
    versions.set_margin_top(16);
    versions.set_margin_bottom(20);
    versions.set_margin_start(20);
    versions.set_margin_end(20);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(&versions));
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));
    populate_history(&versions, target);
    dialog.present(parent);

    let mut rendered = target.edit_history.clone();
    watch_message(&dialog, target, manager, move |message| {
        if message.deleted_for_everyone || message.edit_history.is_empty() {
            return false;
        }
        if rendered != message.edit_history {
            populate_history(&versions, message);
            rendered = message.edit_history.clone();
        }
        true
    });
}

// History and details must stop exposing old content after deletion,
// expiration, local removal, or leaving the account/chat.
pub(super) fn watch_message(
    dialog: &adw::Dialog,
    target: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
    mut update: impl FnMut(&ChatMessageSnapshot) -> bool + 'static,
) {
    let weak = dialog.downgrade();
    let manager = manager.clone();
    let chat_id = target.chat_id.clone();
    let message_id = target.id.clone();
    let source = glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let Some(dialog) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let state = manager.current_state();
        let message = state
            .current_chat
            .as_ref()
            .filter(|chat| chat.chat_id == chat_id)
            .and_then(|chat| {
                chat.messages
                    .iter()
                    .find(|message| message.id == message_id)
            });
        if message.is_none_or(|message| !update(message)) {
            dialog.close();
        }
        glib::ControlFlow::Continue
    });
    let source = RefCell::new(Some(source));
    dialog.connect_closed(move |_| {
        if let Some(source) = source.borrow_mut().take() {
            source.remove();
        }
    });
}

fn populate_history(container: &gtk::Box, message: &ChatMessageSnapshot) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for (index, version) in message.edit_history.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let title = if index == 0 {
            "Original".to_owned()
        } else if index + 1 == message.edit_history.len() {
            "Current".to_owned()
        } else {
            format!("Edit {index}")
        };
        let heading = gtk::Label::new(Some(&title));
        heading.add_css_class("heading");
        heading.set_xalign(0.0);
        row.append(&heading);
        let time =
            glib::DateTime::from_unix_local(version.created_at_secs.min(i64::MAX as u64) as i64)
                .ok()
                .and_then(|time| time.format("%x %X").ok())
                .map(|text| text.to_string())
                .unwrap_or_default();
        let time = gtk::Label::new(Some(&time));
        time.add_css_class("dim-label");
        time.add_css_class("caption");
        time.set_xalign(0.0);
        row.append(&time);
        let body = gtk::Label::new(Some(&version.body));
        body.set_wrap(true);
        body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        body.set_selectable(true);
        body.set_xalign(0.0);
        row.append(&body);
        container.append(&row);
    }
}
