use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, AppState, ChatKind, ChatThreadSnapshot};

use crate::app_manager::AppManager;
use crate::screens::{entry, primary_button, screen_container};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();

    let header = gtk::Label::new(Some("Create group"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let name = entry("Group name");
    container.append(&name);

    let add_member_label = gtk::Label::new(Some("Add members"));
    add_member_label.add_css_class("heading");
    add_member_label.set_halign(gtk::Align::Start);
    add_member_label.set_margin_top(8);
    container.append(&add_member_label);

    let member_input = entry("Search or paste user ID");
    container.append(&member_input);

    let add_member_btn = gtk::Button::with_label("Add member");
    add_member_btn.add_css_class("pill");
    container.append(&add_member_btn);

    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    chips.set_max_children_per_line(20);
    chips.set_row_spacing(6);
    chips.set_column_spacing(6);
    container.append(&chips);

    let members: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

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

    let chips_for_add = chips.clone();
    let input_for_add = member_input.clone();
    let members_for_add = members.clone();
    let add_member = move |raw: String| {
        let value = raw.trim().to_string();
        if value.is_empty() {
            return;
        }
        if members_for_add.borrow().iter().any(|v| v == &value) {
            input_for_add.set_text("");
            return;
        }
        members_for_add.borrow_mut().push(value.clone());
        input_for_add.set_text("");

        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chip.add_css_class("card");
        chip.set_margin_top(2);
        chip.set_margin_bottom(2);

        let label = gtk::Label::new(Some(&shorten(&value)));
        label.set_margin_start(8);
        chip.append(&label);

        let remove = gtk::Button::from_icon_name("window-close-symbolic");
        remove.add_css_class("flat");
        remove.add_css_class("circular");
        chip.append(&remove);

        let chip_for_remove = chip.clone();
        let chips_for_remove = chips_for_add.clone();
        let members_for_remove = members_for_add.clone();
        let value_for_remove = value.clone();
        remove.connect_clicked(move |_| {
            members_for_remove
                .borrow_mut()
                .retain(|v| v != &value_for_remove);
            if let Some(parent) = chip_for_remove.parent() {
                if let Some(flow_child) = parent.downcast_ref::<gtk::FlowBoxChild>() {
                    chips_for_remove.remove(flow_child);
                }
            }
        });

        chips_for_add.append(&chip);
    };

    let add_member_for_btn = add_member.clone();
    let input_for_btn = member_input.clone();
    add_member_btn.connect_clicked(move |_| add_member_for_btn(input_for_btn.text().to_string()));
    let add_member_for_enter = add_member.clone();
    member_input.connect_activate(move |entry| add_member_for_enter(entry.text().to_string()));

    if !known_users.is_empty() {
        let known_label = gtk::Label::new(Some("Known users"));
        known_label.add_css_class("heading");
        known_label.set_halign(gtk::Align::Start);
        known_label.set_margin_top(12);
        container.append(&known_label);

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let list_widget: gtk::Widget = list.clone().upcast();
        container.append(&list_widget);

        let mut row_widgets: Vec<adw::ActionRow> = Vec::with_capacity(known_users.len());
        for chat in &known_users {
            let row = known_user_row(chat, add_member.clone());
            list.append(&row);
            row_widgets.push(row);
        }

        let known_users_for_filter = known_users.clone();
        member_input.connect_changed(move |entry| {
            let query = entry.text().to_lowercase();
            let trimmed = query.trim();
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

    let busy = state.busy.creating_group;
    let create = primary_button(if busy { "Creating…" } else { "Create group" });
    create.set_sensitive(!busy);
    create.set_margin_top(8);
    container.append(&create);

    let manager_for_create = manager.clone();
    let name_for_create = name.clone();
    let members_for_create = members.clone();
    create.connect_clicked(move |btn| {
        let group_name = name_for_create.text().trim().to_string();
        if group_name.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        manager_for_create.dispatch(AppAction::CreateGroup {
            name: group_name,
            member_inputs: members_for_create.borrow().clone(),
        });
    });

    container.upcast()
}

fn known_user_row<F>(chat: &ChatThreadSnapshot, add_member: F) -> adw::ActionRow
where
    F: Fn(String) + Clone + 'static,
{
    let title = if chat.display_name.trim().is_empty() {
        chat.chat_id.clone()
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

fn shorten(value: &str) -> String {
    if value.len() <= 18 {
        return value.to_string();
    }
    format!("{}…{}", &value[..10], &value[value.len() - 6..])
}
