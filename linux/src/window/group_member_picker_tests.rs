use super::*;
use iris_chat_core::{ChatKind, GroupDetailsSnapshot, GroupMemberSnapshot};

fn named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == name {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(scroll) = named(&widget, name) {
            return Some(scroll);
        }
        child = widget.next_sibling();
    }
    None
}

pub(super) fn run(manager: &Rc<AppManager>, slot: &Content, header: &HeaderWidgets) {
    let original = manager.current_state();
    let mut state = original.clone();
    let template = original.chat_list.first().expect("fixture contact");
    state.chat_list = (0..21)
        .map(|index| {
            let mut contact = template.clone();
            contact.chat_id = format!("{:064x}", 10_000 + index);
            contact.kind = ChatKind::Direct;
            contact.display_name = format!("Person {index}");
            contact.profile_name = Some(contact.display_name.clone());
            contact.nickname = None;
            contact.picture_url = None;
            contact.subtitle = None;
            contact
        })
        .collect();
    state.group_details = Some(GroupDetailsSnapshot {
        group_id: "picker-test".into(),
        name: "People".into(),
        picture_url: None,
        about: None,
        created_by_display_name: "You".into(),
        created_by_npub: String::new(),
        can_manage: true,
        is_muted: false,
        revision: 1,
        members: vec![],
    });
    state.router.screen_stack = vec![Screen::GroupDetails {
        group_id: "picker-test".into(),
    }];
    let apply = |mut next: AppState| {
        next.rev = manager.current_state().rev + 1;
        assert!(manager.apply_update(AppUpdate::FullState(next)).is_some());
        apply_state(slot, header, manager, &manager.current_state());
    };
    for added in 0..=2 {
        apply(state.clone());
        let scroll = named(slot.root.upcast_ref(), "group-member-candidates")
            .expect("member candidates")
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        assert_eq!(scroll.max_content_height(), 360);
        let list = named(scroll.upcast_ref(), "group-member-candidate-list")
            .unwrap()
            .downcast::<gtk::ListBox>()
            .unwrap();
        let mut count = 0;
        while let Some(row) = list.row_at_index(count) {
            assert!(
                row.is_visible(),
                "no candidate is hidden by the old eight-row limit"
            );
            count += 1;
        }
        assert_eq!(count, 21 - added);
        assert!(find_label(slot.root.upcast_ref(), "Person 20").is_some());
        if added < 2 {
            let index = if added == 0 { 19 } else { 17 };
            let contact = &state.chat_list[index];
            state
                .group_details
                .as_mut()
                .unwrap()
                .members
                .push(GroupMemberSnapshot {
                    social_connection: contact.social_connection.clone(),
                    owner_pubkey_hex: contact.chat_id.clone(),
                    display_name: contact.display_name.clone(),
                    npub: String::new(),
                    picture_url: None,
                    is_admin: false,
                    is_creator: false,
                    is_local_owner: false,
                });
        }
    }
    state.group_details.as_mut().unwrap().can_manage = false;
    apply(state);
    assert!(
        named(slot.root.upcast_ref(), "group-member-candidates").is_none(),
        "current authorization gates member choices"
    );
    apply(original);
    println!("PASS: GTK full member list, bounded scroll and sequential authoritative membership refresh");
}
