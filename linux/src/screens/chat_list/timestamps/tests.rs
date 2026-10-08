use super::*;
use crate::app_manager::AppManager;
use crate::screens::chat_list::{message_hit_row, ChatListView};
use iris_chat_core::{ChatKind, MessageSearchHit};
use std::time::Instant;

fn pump_until(mut ready: impl FnMut() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "timestamp UI did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub fn verify_ui(manager: Rc<AppManager>) {
    let original_query = manager.search_ui().query;
    assert!(original_query.is_empty());
    assert!(manager.search_ui().scope_chat_id.is_none());
    let now = crate::screens::chat_list::unix_now();
    let mut state = manager.current_state();
    let chat = state.chat_list.first_mut().expect("timestamp fixture chat");
    chat.last_message_at_secs = Some(now - 59);
    let chat_id = chat.chat_id.clone();
    let mut view = ChatListView::new(&manager);
    view.update(&state, &manager);
    let body = view.scrolled.child().unwrap();
    let time = view
        .timestamps
        .labels
        .borrow()
        .iter()
        .find(|(timestamp, _)| *timestamp == now - 59)
        .and_then(|(_, weak)| weak.upgrade())
        .unwrap();
    let search_row = message_hit_row(
        &MessageSearchHit {
            chat_id,
            message_id: "timestamp-hit".into(),
            chat_display_name: "Alex".into(),
            chat_picture_url: None,
            chat_kind: ChatKind::Direct,
            author_pubkey: "".into(),
            author_display_name: "Alex".into(),
            author_picture_url: None,
            body: "Earlier search result".into(),
            is_outgoing: false,
            created_at_secs: now - 3600,
        },
        &state.preferences,
        now,
        &manager,
        &view.timestamps,
    );
    view.root.append(&search_row);
    let search_time = view
        .timestamps
        .labels
        .borrow()
        .last()
        .unwrap()
        .1
        .upgrade()
        .unwrap();
    let window = adw::Window::builder()
        .default_width(500)
        .default_height(500)
        .build();
    window.set_content(Some(&view.root));
    window.present();
    pump_until(|| view.entry.is_mapped());
    view.entry.set_text("still entering a search");
    view.entry.grab_focus();
    view.entry.select_region(6, 14);
    pump_until(|| {
        gtk::prelude::GtkWindowExt::focus(&window)
            .is_some_and(|focus| focus.is_ancestor(&view.entry))
    });
    let focus = gtk::prelude::GtkWindowExt::focus(&window);
    view.timestamps.refresh(now);
    assert_eq!(time.text(), "now");
    assert_eq!(search_time.text(), "1h");
    view.timestamps.refresh(now + 61);
    assert_eq!(time.text(), "2m");
    view.timestamps.refresh(now + 3600);
    assert_eq!(search_time.text(), "2h");
    assert_eq!(view.scrolled.child().unwrap(), body);
    assert_eq!(view.entry.text(), "still entering a search");
    assert_eq!(view.entry.selection_bounds(), Some((6, 14)));
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
    assert!(time.parent().is_some() && search_time.parent().is_some());

    gtk::prelude::WidgetExt::display(&window)
        .primary_clipboard()
        .set_content(None::<&gtk::gdk::ContentProvider>)
        .unwrap();
    manager.set_search_query(original_query);
    window.set_content(None::<&gtk::Widget>);
    drop((view, body, search_row, time, search_time));
    window.destroy();
    println!("PASS: chat and search timestamps refresh in place without losing the search editor or focus");
}
