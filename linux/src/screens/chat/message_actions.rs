use super::*;

mod hover;
pub(super) use hover::install_hover_actions;
#[cfg(feature = "ui-tests")]
pub use hover::verify_ui;
#[cfg(feature = "ui-tests")]
mod history_tests;
#[cfg(feature = "ui-tests")]
pub use history_tests::verify_edit_history_ui;

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
        history.set_widget_name("messageEditHistoryMenu");
        history.add_css_class("flat");
        let target = message.clone();
        let manager = manager.clone();
        let account = account_identity(&manager.current_state());
        let popover = popover.clone();
        history.connect_clicked(move |button| {
            if account.is_none() || account_identity(&manager.current_state()) != account {
                popover.popdown();
                return;
            }
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
    button.show_pointer_cursor();
    button.add_css_class("flat");
    button.add_css_class("caption");
    button.set_tooltip_text(Some("Edit history"));
    let target = message.clone();
    let manager = manager.clone();
    let account = account_identity(&manager.current_state());
    button.connect_clicked(move |button| {
        if account.is_none() || account_identity(&manager.current_state()) != account {
            return;
        }
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_history(parent.as_ref(), &target, &manager);
    });
    button
}

pub(super) fn present_history(
    parent: Option<&gtk::Window>,
    target: &ChatMessageSnapshot,
    manager: &Rc<AppManager>,
) {
    let state = manager.current_state();
    let Some((_, target)) = live_message(&state, target) else {
        return;
    };
    if target.deleted_for_everyone || target.edit_history.is_empty() {
        return;
    }
    let dialog = adw::Dialog::builder()
        .title("Edit history")
        .content_width(440)
        .content_height(500)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let versions = gtk::Box::new(gtk::Orientation::Vertical, 16);
    versions.set_widget_name("editHistoryVersions");
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
    crate::widgets::dialogs::present(&dialog, parent);

    let mut rendered = (target.body.clone(), target.edit_history.clone());
    watch_message(&dialog, target, manager, move |message| {
        if message.deleted_for_everyone || message.edit_history.is_empty() {
            return false;
        }
        if rendered.0 != message.body || rendered.1 != message.edit_history {
            populate_history(&versions, message);
            rendered = (message.body.clone(), message.edit_history.clone());
        }
        true
    });
}

pub(super) fn account_identity(state: &AppState) -> Option<(String, String)> {
    state.account.as_ref().map(|account| {
        (
            account.public_key_hex.clone(),
            account.device_public_key_hex.clone(),
        )
    })
}

pub(super) fn live_message<'a>(
    state: &'a AppState,
    target: &ChatMessageSnapshot,
) -> Option<(&'a CurrentChatSnapshot, &'a ChatMessageSnapshot)> {
    state.account.as_ref()?;
    let screen = state
        .router
        .screen_stack
        .last()
        .unwrap_or(&state.router.default_screen);
    if !matches!(
        screen,
        iris_chat_core::Screen::Chat { chat_id }
            | iris_chat_core::Screen::DirectChatInfo { chat_id }
            if chat_id == &target.chat_id
    ) {
        return None;
    }
    let chat = state
        .current_chat
        .as_ref()
        .filter(|chat| chat.chat_id == target.chat_id)?;
    let message = chat.messages.iter().find(|message| {
        message.id == target.id
            && message.chat_id == target.chat_id
            && message
                .expires_at_secs
                .is_none_or(|expires| expires > unix_now())
    })?;
    Some((chat, message))
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
    let target = target.clone();
    let account = account_identity(&manager.current_state());
    let mut invalidated = false;
    let source = glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let Some(dialog) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !invalidated {
            let state = manager.current_state();
            let same_account = account.is_some() && account_identity(&state) == account;
            let message = same_account
                .then(|| live_message(&state, &target))
                .flatten()
                .map(|(_, message)| message);
            invalidated = message.is_none_or(|message| !update(message));
        }
        // libadwaita 1.5 can reopen a sheet closed before its content maps.
        // Keep the invalidation and watcher until the pending open finishes.
        if invalidated && dialog.child().is_some_and(|content| content.is_mapped()) {
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
    let mut existing = std::collections::HashMap::new();
    let mut child = container.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        existing.insert(row.widget_name().to_string(), row);
    }
    let mut previous = None::<gtk::Widget>;
    for (index, version) in message.edit_history.iter().enumerate().rev() {
        let name = format!("editHistoryVersion-{}", version.id);
        let row = existing.remove(&name).unwrap_or_else(|| {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            row.set_widget_name(&name);
            let heading = gtk::Label::new(None);
            heading.add_css_class("heading");
            heading.set_xalign(0.0);
            row.append(&heading);
            let time = gtk::Label::new(None);
            time.add_css_class("dim-label");
            time.add_css_class("caption");
            time.set_selectable(true);
            time.set_xalign(0.0);
            row.append(&time);
            let body = gtk::Label::new(None);
            body.set_wrap(true);
            body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            body.set_selectable(true);
            body.set_xalign(0.0);
            row.append(&body);
            container.append(&row);
            row.upcast()
        });
        let title = if index + 1 == message.edit_history.len() {
            "Current".to_owned()
        } else if index == 0 {
            "Original".to_owned()
        } else {
            format!("Edit {index}")
        };
        let time =
            glib::DateTime::from_unix_local(version.created_at_secs.min(i64::MAX as u64) as i64)
                .ok()
                .and_then(|time| time.format("%x %X").ok())
                .map(|text| text.to_string())
                .unwrap_or_default();
        let mut label = row.first_child();
        for text in [
            title.as_str(),
            time.as_str(),
            reply_stripped_body(&version.body),
        ] {
            let current = label.unwrap().downcast::<gtk::Label>().unwrap();
            label = current.next_sibling();
            // Version ids are stable. Leave unchanged labels and selection intact.
            if current.text() != text {
                current.set_text(text);
            }
        }
        container.reorder_child_after(&row, previous.as_ref());
        previous = Some(row);
    }
    for row in existing.into_values() {
        if let Some(window) = container.root().and_downcast::<gtk::Window>() {
            if gtk::prelude::GtkWindowExt::focus(&window)
                .is_some_and(|focused| focused == row || focused.is_ancestor(&row))
            {
                // A removed revision cannot retain selection; focus the stable
                // scrolling control before GTK releases the selectable label.
                if container
                    .ancestor(gtk::ScrolledWindow::static_type())
                    .is_none_or(|scroll| !scroll.grab_focus())
                {
                    gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
                }
            }
        }
        container.remove(&row);
    }
}
