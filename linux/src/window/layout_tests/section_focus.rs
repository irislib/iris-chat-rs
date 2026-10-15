use super::*;

fn focus_state(
    slot: &Content,
    manager: &Rc<AppManager>,
    window: &adw::Window,
    chat_id: &str,
) -> String {
    let state = manager.current_state();
    let root = slot.root.upcast_ref::<gtk::Widget>();
    let search = named(root, "iris-keyboard-search");
    let input = text_view(root);
    let row = descendants(root)
        .into_iter()
        .find(|widget| widget.widget_name() == format!("iris-keyboard-chat-{chat_id}"));
    let focused = gtk::prelude::GtkWindowExt::focus(window);
    format!(
        "list_route={} chat_route={} search_mapped={} search_focused={} row_mapped={} row_focused={} composer_mapped={} composer_focused={} focused_type={:?} {}",
        current_screen(&state) == Screen::ChatList,
        matches!(current_screen(&state), Screen::Chat { .. }),
        search.is_mapped(), owns_focus(window, &search),
        row.as_ref().is_some_and(|row| row.is_mapped()),
        row.as_ref().is_some_and(|row| owns_focus(window, row)),
        input.as_ref().is_some_and(|input| input.is_mapped()),
        input.as_ref().is_some_and(|input| input.has_focus()),
        focused.as_ref().map(|widget| widget.type_().name()),
        slot.section_focus_debug(),
    )
}

pub(super) fn check_narrow_section_shortcuts(
    slot: &Content,
    header: &HeaderWidgets,
    manager: &Rc<AppManager>,
    window: &adw::Window,
    chat_id: &str,
) {
    let wait = |stage: &str, mut ready: Box<dyn FnMut() -> bool + '_>| {
        let mut previous = String::new();
        pump_until(|| {
            let result = ready();
            let state = focus_state(slot, manager, window, chat_id);
            if state != previous {
                eprintln!("SECTION_FOCUS {stage}: {state}");
                previous = state;
            }
            result
        });
    };
    let rx = manager.update_rx();
    let drain = || {
        while let Ok(update) = rx.try_recv() {
            if let Some(AppUpdate::FullState(state)) = manager.apply_update(update) {
                apply_state(slot, header, manager, &state);
            }
        }
    };
    // Use real navigation updates here; synthetic layout snapshots above do
    // not change the manager's route or the underlying conversation.
    manager.dispatch(AppAction::OpenChat {
        chat_id: chat_id.into(),
    });
    apply_state(slot, header, manager, &manager.current_state());
    wait(
        "open-chat",
        Box::new(|| {
            drain();
            text_view(slot.root.upcast_ref()).is_some_and(|input| input.is_mapped())
                && matches!(current_screen(&manager.current_state()), Screen::Chat { chat_id: id } if id == chat_id)
        }),
    );
    let keys = window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("production section keyboard shortcuts");
    let press = |modifiers: gtk::gdk::ModifierType| {
        eprintln!(
            "SECTION_KEY {modifiers:?} before: {}",
            focus_state(slot, manager, window, chat_id)
        );
        assert!(keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::t, &0u32, &modifiers]));
        eprintln!(
            "SECTION_KEY {modifiers:?} after: {}",
            focus_state(slot, manager, window, chat_id)
        );
    };
    press(gtk::gdk::ModifierType::CONTROL_MASK);
    wait(
        "chat-list",
        Box::new(|| {
            drain();
            if current_screen(&manager.current_state()) != Screen::ChatList {
                return false;
            }
            let row = named(
                slot.root.upcast_ref(),
                &format!("iris-keyboard-chat-{chat_id}"),
            );
            row.is_mapped() && owns_focus(window, &row)
        }),
    );
    press(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
    wait(
        "composer",
        Box::new(|| {
            drain();
            matches!(current_screen(&manager.current_state()), Screen::Chat { chat_id: id } if id == chat_id)
                && text_view(slot.root.upcast_ref())
                    .is_some_and(|input| input.is_mapped() && input.has_focus())
        }),
    );

    let original_query = manager.search_ui().query;
    manager.set_search_query("layout-empty-results-83ad0927".into());
    apply_state(slot, header, manager, &manager.current_state());
    pump_until(|| {
        drain();
        manager.search_results(50).is_some_and(|results| {
            results.people.is_empty()
                && results.contacts.is_empty()
                && results.groups.is_empty()
                && results.messages.is_empty()
                && results.shortcut.is_none()
        })
    });
    apply_state(slot, header, manager, &manager.current_state());
    press(gtk::gdk::ModifierType::CONTROL_MASK);
    wait(
        "empty-search",
        Box::new(|| {
            drain();
            let search = named(slot.root.upcast_ref(), "iris-keyboard-search");
            current_screen(&manager.current_state()) == Screen::ChatList
                && search.is_mapped()
                && owns_focus(window, &search)
        }),
    );
    let list = named(slot.root.upcast_ref(), "iris-keyboard-chat-list");
    assert!(
        !descendants(&list)
            .iter()
            .any(|widget| widget.is::<adw::ActionRow>() || widget.is::<gtk::Button>()),
        "empty-result shortcut must focus search without requiring a result row"
    );
    press(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
    wait(
        "composer",
        Box::new(|| {
            drain();
            matches!(current_screen(&manager.current_state()), Screen::Chat { chat_id: id } if id == chat_id)
                && text_view(slot.root.upcast_ref())
                    .is_some_and(|input| input.is_mapped() && input.has_focus())
        }),
    );
    manager.set_search_query(original_query);
    apply_state(slot, header, manager, &manager.current_state());
    assert_eq!(
        manager.current_state().current_chat.unwrap().chat_id,
        chat_id
    );
}
