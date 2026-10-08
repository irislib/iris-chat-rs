use super::*;
use iris_chat_core::{DeviceHistorySyncPhase, DeviceHistorySyncSnapshot};

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    result
}

fn named(root: &gtk::Widget, name: &str) -> gtk::Widget {
    descendants(root)
        .into_iter()
        .find(|widget| widget.widget_name() == name)
        .unwrap_or_else(|| panic!("missing widget {name}"))
}

fn owns_focus(window: &adw::Window, widget: &gtk::Widget) -> bool {
    gtk::prelude::GtkWindowExt::focus(window)
        .is_some_and(|focused| focused == *widget || focused.is_ancestor(widget))
}

fn check_preview(root: &gtk::Widget, chat_id: &str) {
    let row = named(root, &format!("iris-keyboard-chat-{chat_id}"))
        .downcast::<adw::ActionRow>()
        .unwrap();
    pump_until(|| row.is_mapped() && row.height() > 0);
    assert_eq!(row.title_lines(), 1);
    assert_eq!(row.subtitle_lines(), 1);
    assert!(!row.subtitle().unwrap().contains('\n'));
    let preview = descendants(row.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .find(|label| label.has_css_class("subtitle"))
        .expect("chat preview label");
    assert_eq!(preview.ellipsize(), gtk::pango::EllipsizeMode::End);
    assert_eq!(preview.layout().line_count(), 1);
    assert!(
        preview.layout().is_ellipsized(),
        "long previews must truncate"
    );
    assert!(
        row.height() < 100,
        "preview expanded row to {}px",
        row.height()
    );
}

fn check_controls(root: &gtk::Widget) {
    for tooltip in ["Add attachment", "Insert emoji", "Send"] {
        let control = descendants(root)
            .into_iter()
            .find(|widget| widget.tooltip_text().as_deref() == Some(tooltip))
            .unwrap_or_else(|| panic!("missing {tooltip} control"));
        let button = if let Ok(menu) = control.clone().downcast::<gtk::MenuButton>() {
            descendants(menu.upcast_ref())
                .into_iter()
                .find(|widget| widget.is::<gtk::ToggleButton>())
                .expect("menu button's visible control")
        } else {
            control.clone()
        };
        pump_until(|| button.width() > 0 && button.height() > 0);
        assert!(control.has_css_class("circular"));
        assert!(
            (button.width() - button.height()).abs() <= 1,
            "{tooltip} must stay circular with multiline text: {}×{}",
            button.width(),
            button.height()
        );
        assert!(button.width() >= 40, "{tooltip} has a cramped hit target");
    }
    assert!(find_label(root, "No expiry").is_none());
    assert!(!descendants(root)
        .iter()
        .any(|widget| widget.is::<gtk::DropDown>()));
}

fn check_sidebar_settings(root: &gtk::Widget) {
    let settings = descendants(root)
        .into_iter()
        .find(|widget| widget.tooltip_text().as_deref() == Some("Settings"))
        .expect("sidebar Settings button");
    let avatar = descendants(&settings)
        .into_iter()
        .find_map(|widget| widget.downcast::<adw::Avatar>().ok())
        .expect("Settings button must contain the account avatar");
    pump_until(|| avatar.is_mapped() && avatar.width() > 0 && avatar.height() > 0);
    assert!(settings.is_mapped());
    assert!(avatar.shows_initials());
    assert!(avatar.text().is_some_and(|text| !text.is_empty()));
    assert!(avatar.width() >= 28 && avatar.height() >= 28);
}

fn check_new_chat_form(
    slot: &Content,
    header: &HeaderWidgets,
    manager: &Rc<AppManager>,
    window: &adw::Window,
    state: &AppState,
) {
    let mut state = state.clone();
    state.router.screen_stack = vec![Screen::NewChat];
    apply_state(slot, header, manager, &state);
    let peer = descendants(slot.detail.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Entry>().ok())
        .find(|entry| entry.placeholder_text().as_deref() == Some("User ID or invite"))
        .expect("new-chat user input");
    pump_until(|| peer.is_mapped());
    peer.grab_focus();
    peer.set_text("Still entering a user ID");
    peer.select_region(6, 14);
    pump_until(|| owns_focus(window, peer.upcast_ref()));
    state.busy.syncing_network = !state.busy.syncing_network;
    state.user_discovery_syncing = !state.user_discovery_syncing;
    apply_state(slot, header, manager, &state);
    assert!(
        peer.root().is_some(),
        "background sync replaced the new-chat form"
    );
    assert_eq!(peer.text(), "Still entering a user ID");
    assert_eq!(peer.selection_bounds(), Some((6, 14)));
    assert!(owns_focus(window, peer.upcast_ref()));
}

fn check_narrow_section_shortcuts(
    slot: &Content,
    header: &HeaderWidgets,
    manager: &Rc<AppManager>,
    window: &adw::Window,
    chat_id: &str,
) {
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
    pump_until(|| {
        drain();
        text_view(slot.root.upcast_ref()).is_some_and(|input| input.is_mapped())
            && matches!(current_screen(&manager.current_state()), Screen::Chat { chat_id: id } if id == chat_id)
    });
    let keys = window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("production section keyboard shortcuts");
    let press = |modifiers: gtk::gdk::ModifierType| {
        assert!(keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::t, &0u32, &modifiers]));
    };
    press(gtk::gdk::ModifierType::CONTROL_MASK);
    pump_until(|| {
        drain();
        if current_screen(&manager.current_state()) != Screen::ChatList {
            return false;
        }
        let row = named(
            slot.root.upcast_ref(),
            &format!("iris-keyboard-chat-{chat_id}"),
        );
        row.is_mapped() && owns_focus(window, &row)
    });
    press(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
    pump_until(|| {
        drain();
        matches!(current_screen(&manager.current_state()), Screen::Chat { chat_id: id } if id == chat_id)
            && text_view(slot.root.upcast_ref())
                .is_some_and(|input| input.is_mapped() && input.has_focus())
    });

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
    pump_until(|| {
        drain();
        let search = named(slot.root.upcast_ref(), "iris-keyboard-search");
        current_screen(&manager.current_state()) == Screen::ChatList
            && search.is_mapped()
            && owns_focus(window, &search)
    });
    let list = named(slot.root.upcast_ref(), "iris-keyboard-chat-list");
    assert!(
        !descendants(&list)
            .iter()
            .any(|widget| widget.is::<adw::ActionRow>() || widget.is::<gtk::Button>()),
        "empty-result shortcut must focus search without requiring a result row"
    );
    press(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
    pump_until(|| {
        drain();
        matches!(current_screen(&manager.current_state()), Screen::Chat { chat_id: id } if id == chat_id)
            && text_view(slot.root.upcast_ref())
                .is_some_and(|input| input.is_mapped() && input.has_focus())
    });
    manager.set_search_query(original_query);
    apply_state(slot, header, manager, &manager.current_state());
    assert_eq!(
        manager.current_state().current_chat.unwrap().chat_id,
        chat_id
    );
}

fn screenshot(window: &adw::Window, filename: &str) {
    let Some(directory) = std::env::var_os("IRIS_LAYOUT_UI_SCREENSHOTS") else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let mut node = None;
    pump_until(|| {
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        node = snapshot.to_node();
        node.is_some()
    });
    window
        .renderer()
        .unwrap()
        .render_texture(node.as_ref().unwrap(), None)
        .save_to_png(std::path::Path::new(&directory).join(filename))
        .unwrap();
}

pub(super) fn run(manager: Rc<AppManager>) {
    let original_query = manager.search_ui().query;
    let mut state = manager.current_state();
    state.busy.syncing_network = false;
    state.device_history_sync = None;
    let chat = state.current_chat.as_mut().expect("fixture chat");
    chat.direct_chat_capability = None;
    chat.draft = "A message with several lines\nto check the send and emoji buttons\nwhile keeping the editor steady.".into();
    let mut message = iris_chat_core::ChatMessageSnapshot {
        id: "hover-actions".into(),
        chat_id: chat.chat_id.clone(),
        kind: iris_chat_core::ChatMessageKind::User,
        author: "Alex".into(),
        author_owner_pubkey_hex: None,
        author_picture_url: None,
        body: "See you at the park!".into(),
        edit_history: vec![],
        deleted_for_everyone: false,
        system_notice_owner_pubkey_hex: None,
        attachments: vec![],
        direct_transfer: None,
        reactions: vec![
            iris_chat_core::MessageReactionSnapshot {
                emoji: "❤️".into(),
                count: 2,
                reacted_by_me: true,
            },
            iris_chat_core::MessageReactionSnapshot {
                emoji: "👍".into(),
                count: 1,
                reacted_by_me: false,
            },
        ],
        reactors: vec![],
        is_outgoing: false,
        created_at_secs: crate::screens::chat_list::unix_now(),
        expires_at_secs: None,
        delivery: iris_chat_core::DeliveryState::Seen,
        recipient_deliveries: vec![],
        delivery_trace: Default::default(),
        source_event_id: None,
        call: None,
    };
    chat.display_name = "Alex".into();
    chat.nickname = Some("Alex".into());
    chat.messages = vec![message.clone()];
    message.id = "layout-outgoing".into();
    message.is_outgoing = true;
    message.body = "Sounds good, see you there!".into();
    chat.messages.push(message);
    let chat_id = chat.chat_id.clone();
    let preview = state
        .chat_list
        .iter_mut()
        .find(|chat| chat.chat_id == chat_id)
        .unwrap();
    preview.display_name = "Alex".into();
    preview.nickname = Some("Alex".into());
    preview.draft.clear();
    preview.is_typing = false;
    preview.last_message_preview = Some(
        "A long message preview should use one line.\nSecond line should not make this row grow. "
            .repeat(8),
    );

    let slot = Content::new();
    let (bar, header) = build_header(&manager);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&bar);
    slot.install_toolbar(&toolbar, &bar, &header.back);
    toolbar.set_content(Some(&slot.detail));
    let window = adw::Window::builder()
        .default_width(1080)
        .default_height(740)
        .build();
    window.set_content(Some(&slot.root));
    slot.install_section_shortcuts(&window, &manager);
    apply_state(&slot, &header, &manager, &state);
    window.present();
    let root = slot.root.upcast_ref::<gtk::Widget>();
    let input = text_view(root).unwrap();
    let search = named(root, "iris-keyboard-search")
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    let split = descendants(root)
        .into_iter()
        .find_map(|widget| widget.downcast::<adw::NavigationSplitView>().ok())
        .unwrap();
    pump_until(|| window.width() >= 1000 && input.is_mapped() && search.is_mapped());
    assert!(!split.is_collapsed());
    assert!(
        !header.back.is_visible(),
        "desktop keeps the chat list beside the conversation"
    );
    check_preview(root, &chat_id);
    check_controls(root);
    check_sidebar_settings(root);

    search.grab_focus();
    pump_until(|| owns_focus(&window, search.upcast_ref()));
    // Apply a background update before SearchEntry's debounced search signal.
    search.set_text("background search");
    search.select_region(2, 7);
    state.busy.syncing_network = true;
    apply_state(&slot, &header, &manager, &state);
    assert_eq!(
        named(root, "iris-keyboard-search"),
        search.clone().upcast::<gtk::Widget>()
    );
    assert_eq!(search.text(), "background search");
    assert_eq!(search.selection_bounds(), Some((2, 7)));
    assert!(owns_focus(&window, search.upcast_ref()));
    let statuses: Vec<_> = descendants(root)
        .into_iter()
        .filter(|widget| widget.widget_name() == "message-sync-status")
        .collect();
    assert!(!statuses.is_empty());
    assert!(statuses.iter().all(|status| status.is_visible()));
    assert!(find_label(root, "Syncing messages…").is_some());
    state.busy.syncing_network = false;
    state.device_history_sync = Some(DeviceHistorySyncSnapshot {
        phase: DeviceHistorySyncPhase::Waiting,
        imported_messages: 3,
        total_messages: Some(12),
    });
    apply_state(&slot, &header, &manager, &state);
    assert!(find_label(root, "Waiting for your other device…").is_some());
    state.device_history_sync.as_mut().unwrap().phase = DeviceHistorySyncPhase::Transferring;
    apply_state(&slot, &header, &manager, &state);
    assert!(find_label(root, "Syncing messages… 3 of 12").is_some());
    screenshot(&window, "linux-syncing.png");
    state.device_history_sync.as_mut().unwrap().phase = DeviceHistorySyncPhase::Complete;
    apply_state(&slot, &header, &manager, &state);
    assert!(statuses.iter().all(|status| !status.is_visible()));
    assert_eq!(search.selection_bounds(), Some((2, 7)));
    assert!(owns_focus(&window, search.upcast_ref()));

    search.set_text(&original_query);
    manager.set_search_query(original_query);
    apply_state(&slot, &header, &manager, &state);
    input.grab_focus();
    input.buffer().select_range(
        &input.buffer().iter_at_offset(2),
        &input.buffer().iter_at_offset(9),
    );
    pump_until(|| input.has_focus());
    screenshot(&window, "linux-chat-wide.png");
    window.set_default_size(390, 740);
    pump_until(|| window.width() < 760 && split.is_collapsed() && input.is_mapped());
    assert!(input.is_mapped());
    assert!(!search.is_mapped(), "narrow chat uses the full window");
    assert!(header.back.is_visible());
    assert_eq!(text_view(root).unwrap(), input);
    pump_until(|| input.buffer().selection_bounds().is_some());
    let (start, end) = input.buffer().selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (2, 9));
    check_controls(root);
    screenshot(&window, "linux-chat-narrow.png");
    window.set_default_size(1080, 740);
    pump_until(|| !split.is_collapsed() && search.is_mapped());
    assert_eq!(text_view(root).unwrap(), input);
    assert_eq!(
        named(root, "iris-keyboard-search"),
        search.clone().upcast::<gtk::Widget>()
    );
    check_preview(root, &chat_id);

    state.router.screen_stack.clear();
    apply_state(&slot, &header, &manager, &state);
    window.set_default_size(390, 740);
    pump_until(|| split.is_collapsed() && search.is_mapped());
    check_preview(root, &chat_id);
    screenshot(&window, "linux-chat-list-narrow.png");
    check_sidebar_settings(root);
    eprintln!("Checking persistent new-chat form");
    check_new_chat_form(&slot, &header, &manager, &window, &state);
    eprintln!("Checking narrow keyboard navigation");
    check_narrow_section_shortcuts(&slot, &header, &manager, &window, &chat_id);
    let mut settings_state = manager.current_state();
    settings_state.router.screen_stack.push(Screen::Settings);
    apply_state(&slot, &header, &manager, &settings_state);
    let settings_navigation = named(root, "settings-navigation");
    pump_until(|| settings_navigation.is_mapped());
    settings_state.busy.syncing_network = !settings_state.busy.syncing_network;
    apply_state(&slot, &header, &manager, &settings_state);
    assert_eq!(
        named(root, "settings-navigation"),
        settings_navigation,
        "background updates must preserve the Settings view"
    );
    screenshot(&window, "linux-settings-shell-narrow.png");
    window.destroy();
}
