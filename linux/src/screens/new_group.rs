use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState};

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

    let member_input = entry("npub of a member");
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

    let chips_for_add = chips.clone();
    let input_for_add = member_input.clone();
    let members_for_add = members.clone();
    let add_member = move || {
        let value = input_for_add.text().trim().to_string();
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
            members_for_remove.borrow_mut().retain(|v| v != &value_for_remove);
            if let Some(parent) = chip_for_remove.parent() {
                if let Some(flow_child) = parent.downcast_ref::<gtk::FlowBoxChild>() {
                    chips_for_remove.remove(flow_child);
                }
            }
        });

        chips_for_add.append(&chip);
    };

    let add_member_for_btn = add_member.clone();
    add_member_btn.connect_clicked(move |_| add_member_for_btn());
    let add_member_for_enter = add_member.clone();
    member_input.connect_activate(move |_| add_member_for_enter());

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

fn shorten(value: &str) -> String {
    if value.len() <= 18 {
        return value.to_string();
    }
    format!("{}…{}", &value[..10], &value[value.len() - 6..])
}
