use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{
    is_valid_peer_input, normalize_peer_input, AppAction, AppState, ChatKind, ChatThreadSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Select members"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let members_step = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let details_step = gtk::Box::new(gtk::Orientation::Vertical, 12);
    details_step.set_visible(false);
    container.append(&members_step);
    container.append(&details_step);

    let selected_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    members_step.append(&selected_list);

    let member_input = entry("Search or paste user ID");
    members_step.append(&member_input);

    let members: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let selected_photo: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    let local_owner = state
        .account
        .as_ref()
        .map(|account| account.public_key_hex.clone());

    let known_users: Vec<ChatThreadSnapshot> = state
        .chat_list
        .iter()
        .filter(|chat| matches!(chat.kind, ChatKind::Direct))
        .filter(|chat| {
            state
                .account
                .as_ref()
                .map(|a| a.public_key_hex != chat.chat_id)
                .unwrap_or(true)
        })
        .cloned()
        .collect();

    let next = primary_button("Next");

    let add_member = {
        let selected_list = selected_list.clone();
        let input = member_input.clone();
        let members = members.clone();
        let next = next.clone();
        move |raw: String| {
            let value = normalize_peer_input(raw);
            if value.is_empty() || !is_valid_peer_input(value.clone()) {
                return;
            }
            if local_owner.as_deref() == Some(value.as_str()) {
                input.set_text("");
                return;
            }
            if members.borrow().iter().any(|v| v == &value) {
                input.set_text("");
                return;
            }
            members.borrow_mut().push(value.clone());
            input.set_text("");
            next.set_label(&format!("Next ({})", members.borrow().len()));

            let row = selected_member_row(&value, {
                let members = members.clone();
                let next = next.clone();
                move |owner| {
                    members.borrow_mut().retain(|v| v != owner);
                    let count = members.borrow().len();
                    if count == 0 {
                        next.set_label("Next");
                    } else {
                        next.set_label(&format!("Next ({count})"));
                    }
                }
            });
            selected_list.append(&row);
        }
    };

    {
        let add_member = add_member.clone();
        member_input.connect_activate(move |entry| add_member(entry.text().to_string()));
    }
    {
        let add_member = add_member.clone();
        member_input.connect_changed(move |entry| add_member(entry.text().to_string()));
    }

    if !known_users.is_empty() {
        let known_label = gtk::Label::new(Some("Known users"));
        known_label.add_css_class("heading");
        known_label.set_halign(gtk::Align::Start);
        known_label.set_margin_top(12);
        members_step.append(&known_label);

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        members_step.append(&list);

        let mut row_widgets: Vec<adw::ActionRow> = Vec::with_capacity(known_users.len());
        for chat in &known_users {
            let row = known_user_row(chat, add_member.clone());
            list.append(&row);
            row_widgets.push(row);
        }

        let known_users_for_filter = known_users.clone();
        let known_label_for_filter = known_label.clone();
        member_input.connect_changed(move |entry| {
            let query = entry.text().to_lowercase();
            let trimmed = query.trim();
            known_label_for_filter.set_label(if trimmed.is_empty() {
                "Known users"
            } else {
                "Search results"
            });
            for (chat, row) in known_users_for_filter.iter().zip(row_widgets.iter()) {
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

    members_step.append(&next);

    let name = entry("Group name");
    let photo_label = gtk::Label::new(None);
    photo_label.add_css_class("dim-label");
    photo_label.set_halign(gtk::Align::Start);

    let photo = gtk::Button::with_label("Photo");
    photo.add_css_class("pill");
    {
        let selected_photo = selected_photo.clone();
        let photo_label = photo_label.clone();
        photo.connect_clicked(move |btn| {
            let parent = btn
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            let dialog = gtk::FileDialog::builder()
                .title("Choose group photo")
                .accept_label("Choose")
                .build();
            let selected_photo = selected_photo.clone();
            let photo_label = photo_label.clone();
            dialog.open(
                parent.as_ref(),
                gtk::gio::Cancellable::NONE,
                move |result| {
                    let Ok(file) = result else { return };
                    let Some(path) = file.path() else { return };
                    let filename = file
                        .basename()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| "group-photo".to_string());
                    *selected_photo.borrow_mut() =
                        Some((path.to_string_lossy().to_string(), filename.clone()));
                    photo_label.set_label(&filename);
                },
            );
        });
    }
    details_step.append(&photo);
    details_step.append(&photo_label);
    details_step.append(&name);

    let selected_count = gtk::Label::new(Some("Members (0)"));
    selected_count.add_css_class("dim-label");
    selected_count.set_halign(gtk::Align::Start);
    details_step.append(&selected_count);

    let busy = state.busy.creating_group;
    let create = primary_button(if busy { "Creating..." } else { "Create group" });
    create.set_sensitive(!busy);
    details_step.append(&create);

    {
        let header = header.clone();
        let members_step = members_step.clone();
        let details_step = details_step.clone();
        let name = name.clone();
        let members = members.clone();
        let selected_count = selected_count.clone();
        next.connect_clicked(move |_| {
            header.set_label("Group details");
            members_step.set_visible(false);
            details_step.set_visible(true);
            selected_count.set_label(&format!("Members ({})", members.borrow().len()));
            name.grab_focus();
        });
    }

    let back = gtk::Button::with_label("Back");
    back.add_css_class("pill");
    details_step.prepend(&back);
    {
        let header = header.clone();
        let members_step = members_step.clone();
        let details_step = details_step.clone();
        back.connect_clicked(move |_| {
            header.set_label("Select members");
            details_step.set_visible(false);
            members_step.set_visible(true);
        });
    }

    {
        let manager = manager.clone();
        let name = name.clone();
        let members = members.clone();
        let selected_photo = selected_photo.clone();
        create.connect_clicked(move |btn| {
            let group_name = name.text().trim().to_string();
            if group_name.is_empty() {
                return;
            }
            btn.set_sensitive(false);
            let action = match selected_photo.borrow().clone() {
                Some((file_path, filename)) => AppAction::CreateGroupWithPicture {
                    name: group_name,
                    member_inputs: members.borrow().clone(),
                    picture_file_path: file_path,
                    picture_filename: filename,
                },
                None => AppAction::CreateGroup {
                    name: group_name,
                    member_inputs: members.borrow().clone(),
                },
            };
            manager.dispatch(action);
        });
    }

    container.upcast()
}

fn known_user_row<F>(chat: &ChatThreadSnapshot, add_member: F) -> adw::ActionRow
where
    F: Fn(String) + Clone + 'static,
{
    let title = if chat.display_name.trim().is_empty() {
        "Iris user".to_string()
    } else {
        chat.display_name.clone()
    };
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    if let Some(sub) = chat.subtitle.as_ref().filter(|s| !s.is_empty()) {
        row.set_subtitle(sub);
    }
    let avatar = adw::Avatar::new(32, Some(&chat.display_name), true);
    row.add_prefix(&avatar);
    let plus = gtk::Image::from_icon_name("list-add-symbolic");
    plus.add_css_class("dim-label");
    row.add_suffix(&plus);

    let chat_id = chat.chat_id.clone();
    row.connect_activated(move |_| add_member(chat_id.clone()));
    row
}

fn selected_member_row<F>(value: &str, on_remove: F) -> gtk::Box
where
    F: Fn(&str) + 'static,
{
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("card");
    row.set_margin_top(2);
    row.set_margin_bottom(2);

    let label = gtk::Label::new(Some(&shorten(value)));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    label.set_margin_start(8);
    row.append(&label);

    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.add_css_class("flat");
    remove.add_css_class("circular");
    row.append(&remove);

    let row_for_remove = row.clone();
    let value = value.to_string();
    remove.connect_clicked(move |_| {
        on_remove(&value);
        row_for_remove.unparent();
    });

    row
}

fn shorten(value: &str) -> String {
    if value.len() <= 18 {
        return value.to_string();
    }
    format!("{}...{}", &value[..10], &value[value.len() - 6..])
}
