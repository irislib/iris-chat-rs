use crate::app_manager::AppManager;
use adw::prelude::*;
use iris_chat_core::{AppAction, CurrentChatSnapshot};
use std::rc::Rc;

pub fn name_notice(chat: &CurrentChatSnapshot, manager: &Rc<AppManager>) -> gtk::Box {
    name_notice_with_dialog(chat, manager, None)
}

fn name_notice_with_dialog(
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
    dialog: Option<&adw::Dialog>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if let Some(proposed) = chat
        .contact_identity
        .as_ref()
        .and_then(|contact| contact.pending_name.clone())
    {
        let text = gtk::Label::new(Some(&format!("New profile name: {proposed}")));
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.set_max_width_chars(36);
        text.set_xalign(0.0);
        row.append(&text);
        let approve = gtk::Button::with_label("Use new name");
        approve.set_halign(gtk::Align::Start);
        let manager = manager.clone();
        let owner = chat.chat_id.clone();
        let close = dialog.cloned();
        approve.connect_clicked(move |_| {
            manager.dispatch(AppAction::ApproveContactName {
                owner_pubkey_hex: owner.clone(),
                name: proposed.clone(),
            });
            if let Some(dialog) = &close {
                dialog.close();
            }
        });
        row.append(&approve);
    }
    row
}

pub fn profile(
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
    dialog: &adw::Dialog,
) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let Some(contact) = chat.contact_identity.as_ref() else {
        return column;
    };
    let follow = gtk::Button::with_label(if contact.updating_follow {
        "Saving…"
    } else if contact.is_following {
        "Unfollow (public)"
    } else {
        "Follow (public)"
    });
    follow.set_halign(gtk::Align::Start);
    follow.set_sensitive(contact.can_follow && !contact.updating_follow);
    follow.set_tooltip_text(Some(if contact.can_follow {
        "Visible to everyone"
    } else {
        "Use your main device to change public follows"
    }));
    let (follow_manager, owner, following, close) = (
        manager.clone(),
        chat.chat_id.clone(),
        !contact.is_following,
        dialog.clone(),
    );
    follow.connect_clicked(move |_| {
        follow_manager.dispatch(AppAction::SetPublicFollow {
            owner_pubkey_hex: owner.clone(),
            following,
        });
        close.close();
    });
    column.append(&follow);
    let favorite = gtk::Button::with_label(if contact.is_favorite {
        "★ Favorited"
    } else {
        "☆ Favorite"
    });
    favorite.set_halign(gtk::Align::Start);
    favorite.set_tooltip_text(Some("Only you can see this"));
    let (fav_manager, owner, value, close) = (
        manager.clone(),
        chat.chat_id.clone(),
        !contact.is_favorite,
        dialog.clone(),
    );
    favorite.connect_clicked(move |_| {
        fav_manager.dispatch(AppAction::SetContactFavorite {
            owner_pubkey_hex: owner.clone(),
            favorite: value,
        });
        close.close();
    });
    column.append(&favorite);
    column.append(&gtk::Label::new(Some("Favorites are only visible to you")));
    column.append(&name_notice_with_dialog(chat, manager, Some(dialog)));
    if let Some(first) = contact
        .first_seen_name
        .as_ref()
        .filter(|first| Some(*first) != contact.saved_name.as_ref())
    {
        let label = gtk::Label::new(Some(&format!("First known as {first}")));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(36);
        label.set_xalign(0.0);
        column.append(&label);
    }
    column
}
