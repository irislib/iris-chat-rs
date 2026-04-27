use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{
    proxied_image_url, AppAction, AppState, ChatKind, ChatThreadSnapshot, GroupDetailsSnapshot,
    GroupMemberSnapshot,
};

use crate::app_manager::AppManager;
use crate::widgets::image_cache;

pub fn render(group_id: &str, state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let inner = gtk::Box::new(gtk::Orientation::Vertical, 16);
    inner.set_margin_top(20);
    inner.set_margin_bottom(20);
    inner.set_margin_start(16);
    inner.set_margin_end(16);

    let Some(details) = state
        .group_details
        .as_ref()
        .filter(|d| d.group_id == group_id)
    else {
        let label = gtk::Label::new(Some("Loading group…"));
        label.add_css_class("dim-label");
        label.set_vexpand(true);
        label.set_valign(gtk::Align::Center);
        inner.append(&label);
        scrolled.set_child(Some(&inner));
        return scrolled.upcast();
    };

    inner.append(&settings_card(group_id, details, state, manager));
    inner.append(&members_card(group_id, details, state, manager));
    if details.can_manage {
        inner.append(&add_members_card(group_id, details, state, manager));
    }

    scrolled.set_child(Some(&inner));
    scrolled.upcast()
}

fn settings_card(
    group_id: &str,
    details: &GroupDetailsSnapshot,
    state: &AppState,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Group settings")
        .description(format!("Created by {}. Revision {}.", details.created_by_display_name, details.revision))
        .build();

    let avatar_row = adw::ActionRow::new();
    avatar_row.set_activatable(false);
    let avatar = adw::Avatar::new(48, Some(&details.name), true);
    if let Some(url) = details.picture_url.as_ref() {
        let proxied =
            proxied_image_url(url.clone(), state.preferences.clone(), Some(96), Some(96), true);
        image_cache::fetch_into_avatar(&avatar, &proxied);
    }
    avatar_row.add_prefix(&avatar);
    avatar_row.set_title(&details.name);

    if details.can_manage {
        let change = gtk::Button::with_label("Change photo");
        change.add_css_class("flat");
        change.set_valign(gtk::Align::Center);
        change.set_sensitive(!state.busy.uploading_attachment);
        let manager_for_change = manager.clone();
        let group_id_owned = group_id.to_string();
        change.connect_clicked(move |btn| {
            let parent = btn
                .root()
                .and_then(|r| r.downcast::<gtk::Window>().ok());
            let dialog = gtk::FileDialog::builder()
                .title("Choose group photo")
                .build();
            let manager = manager_for_change.clone();
            let group_id = group_id_owned.clone();
            dialog.open(parent.as_ref(), gtk::gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                let filename = file
                    .basename()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image".to_string());
                manager.dispatch(AppAction::UpdateGroupPicture {
                    group_id: group_id.clone(),
                    file_path: path.to_string_lossy().to_string(),
                    filename,
                });
            });
        });
        avatar_row.add_suffix(&change);
    }

    group.add(&avatar_row);

    let name_row = adw::EntryRow::builder().title("Name").build();
    name_row.set_text(&details.name);

    if details.can_manage {
        let busy = state.busy.updating_group;
        let rename = gtk::Button::with_label(if busy { "Renaming…" } else { "Rename" });
        rename.add_css_class("suggested-action");
        rename.set_valign(gtk::Align::Center);
        rename.set_sensitive(!busy);

        let group_id_for_click = group_id.to_string();
        let manager_for_click = manager.clone();
        let row_for_click = name_row.clone();
        rename.connect_clicked(move |btn| {
            let value = row_for_click.text().trim().to_string();
            if value.is_empty() {
                return;
            }
            btn.set_sensitive(false);
            manager_for_click.dispatch(AppAction::UpdateGroupName {
                group_id: group_id_for_click.clone(),
                name: value,
            });
        });
        name_row.add_suffix(&rename);

        let group_id_for_apply = group_id.to_string();
        let manager_for_apply = manager.clone();
        name_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            if value.is_empty() {
                return;
            }
            manager_for_apply.dispatch(AppAction::UpdateGroupName {
                group_id: group_id_for_apply.clone(),
                name: value,
            });
        });
    } else {
        name_row.set_editable(false);
    }
    group.add(&name_row);

    group.upcast()
}

fn members_card(
    group_id: &str,
    details: &GroupDetailsSnapshot,
    state: &AppState,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Members")
        .description(format!("{} people", details.members.len()))
        .build();

    for member in &details.members {
        group.add(&member_row(group_id, details, member, state, manager));
    }

    group.upcast()
}

fn member_row(
    group_id: &str,
    details: &GroupDetailsSnapshot,
    member: &GroupMemberSnapshot,
    state: &AppState,
    manager: &Rc<AppManager>,
) -> adw::ActionRow {
    let title = if member.display_name.is_empty() {
        "Member".to_string()
    } else {
        member.display_name.clone()
    };
    let row = adw::ActionRow::builder().title(title).build();
    let avatar = adw::Avatar::new(36, Some(&member.display_name), true);
    row.add_prefix(&avatar);

    if member.is_local_owner {
        let pill = gtk::Label::new(Some("You"));
        pill.add_css_class("caption");
        pill.set_valign(gtk::Align::Center);
        row.add_suffix(&pill);
    }
    if member.is_creator {
        let pill = gtk::Label::new(Some("Creator"));
        pill.add_css_class("caption");
        pill.add_css_class("accent");
        pill.set_valign(gtk::Align::Center);
        row.add_suffix(&pill);
    } else if member.is_admin {
        let pill = gtk::Label::new(Some("Admin"));
        pill.add_css_class("caption");
        pill.add_css_class("accent");
        pill.set_valign(gtk::Align::Center);
        row.add_suffix(&pill);
    }

    if details.can_manage && !member.is_local_owner {
        let toggle_admin = gtk::Button::with_label(if member.is_admin {
            "Demote"
        } else {
            "Make admin"
        });
        toggle_admin.add_css_class("flat");
        toggle_admin.set_valign(gtk::Align::Center);
        toggle_admin.set_sensitive(!state.busy.updating_group);
        let manager_for_admin = manager.clone();
        let group_id_owned = group_id.to_string();
        let owner_pubkey_hex = member.owner_pubkey_hex.clone();
        let make_admin = !member.is_admin;
        toggle_admin.connect_clicked(move |_| {
            manager_for_admin.dispatch(AppAction::SetGroupAdmin {
                group_id: group_id_owned.clone(),
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                is_admin: make_admin,
            });
        });
        row.add_suffix(&toggle_admin);

        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove member"));
        remove.set_sensitive(!state.busy.updating_group);
        let manager_for_remove = manager.clone();
        let group_id_owned = group_id.to_string();
        let owner_pubkey_hex = member.owner_pubkey_hex.clone();
        remove.connect_clicked(move |_| {
            manager_for_remove.dispatch(AppAction::RemoveGroupMember {
                group_id: group_id_owned.clone(),
                owner_pubkey_hex: owner_pubkey_hex.clone(),
            });
        });
        row.add_suffix(&remove);
    }

    row
}

fn add_members_card(
    group_id: &str,
    details: &GroupDetailsSnapshot,
    state: &AppState,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Add member").build();

    let entry = adw::EntryRow::builder()
        .title("Search or paste user ID")
        .build();
    let busy = state.busy.updating_group;
    let add = gtk::Button::with_label(if busy { "Adding…" } else { "Add" });
    add.add_css_class("suggested-action");
    add.set_valign(gtk::Align::Center);
    add.set_sensitive(!busy);

    let manager_for_btn = manager.clone();
    let row_for_btn = entry.clone();
    let group_id_owned = group_id.to_string();
    add.connect_clicked(move |btn| {
        let value = row_for_btn.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        row_for_btn.set_text("");
        manager_for_btn.dispatch(AppAction::AddGroupMembers {
            group_id: group_id_owned.clone(),
            member_inputs: vec![value],
        });
    });
    entry.add_suffix(&add);

    let manager_for_apply = manager.clone();
    let group_id_owned = group_id.to_string();
    entry.connect_apply(move |row| {
        let value = row.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        row.set_text("");
        manager_for_apply.dispatch(AppAction::AddGroupMembers {
            group_id: group_id_owned.clone(),
            member_inputs: vec![value],
        });
    });

    group.add(&entry);

    let local_owner_hex = state
        .account
        .as_ref()
        .map(|a| a.public_key_hex.clone())
        .unwrap_or_default();
    let existing_member_hexes: HashSet<String> = details
        .members
        .iter()
        .map(|m| m.owner_pubkey_hex.clone())
        .collect();
    let candidates: Vec<ChatThreadSnapshot> = state
        .chat_list
        .iter()
        .filter(|chat| matches!(chat.kind, ChatKind::Direct))
        .filter(|chat| chat.chat_id != local_owner_hex)
        .filter(|chat| !existing_member_hexes.contains(&chat.chat_id))
        .cloned()
        .collect();

    if !candidates.is_empty() {
        let mut row_widgets: Vec<adw::ActionRow> = Vec::with_capacity(candidates.len());
        for chat in &candidates {
            let title = if chat.display_name.trim().is_empty() {
                chat.chat_id.clone()
            } else {
                chat.display_name.clone()
            };
            let row = adw::ActionRow::builder()
                .title(title)
                .activatable(!busy)
                .build();
            if let Some(sub) = chat.subtitle.as_ref().filter(|s| !s.is_empty()) {
                row.set_subtitle(sub);
            }
            let avatar = adw::Avatar::new(32, Some(&chat.display_name), true);
            row.add_prefix(&avatar);
            let plus = gtk::Image::from_icon_name("list-add-symbolic");
            plus.add_css_class("dim-label");
            row.add_suffix(&plus);

            let manager_for_row = manager.clone();
            let group_id_for_row = group_id.to_string();
            let chat_id_for_row = chat.chat_id.clone();
            row.connect_activated(move |_| {
                manager_for_row.dispatch(AppAction::AddGroupMembers {
                    group_id: group_id_for_row.clone(),
                    member_inputs: vec![chat_id_for_row.clone()],
                });
            });
            group.add(&row);
            row_widgets.push(row);
        }

        let candidates_for_filter = candidates.clone();
        entry.connect_changed(move |entry| {
            let query = entry.text().to_lowercase();
            let trimmed = query.trim();
            for (chat, row) in candidates_for_filter.iter().zip(row_widgets.iter()) {
                let matches = trimmed.is_empty()
                    || chat.display_name.to_lowercase().contains(trimmed)
                    || chat.chat_id.to_lowercase().contains(trimmed)
                    || chat
                        .subtitle
                        .as_ref()
                        .map(|s| s.to_lowercase().contains(trimmed))
                        .unwrap_or(false);
                row.set_visible(matches);
            }
        });
    }

    group.upcast()
}
