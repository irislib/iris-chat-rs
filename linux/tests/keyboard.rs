mod message_timeline;

#[path = "../src/widgets/keyboard_list.rs"]
mod keyboard_list;

use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

fn pump() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn key(body: &gtk::Box, key: gtk::gdk::Key) -> bool {
    let controllers = body.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|i| controllers.item(i))
        .find_map(|item| item.downcast::<gtk::EventControllerKey>().ok())
        .unwrap();
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
    )
}

fn list(with_new_chat: bool) -> (gtk::Box, adw::ActionRow, adw::ActionRow) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let first = adw::ActionRow::builder()
        .title("First chat")
        .activatable(true)
        .build();
    first.set_widget_name("iris-keyboard-chat-first");
    let second = adw::ActionRow::builder()
        .title("Second chat")
        .activatable(true)
        .build();
    second.set_widget_name("iris-keyboard-chat-second");
    body.set_widget_name("iris-keyboard-chat-list");
    if with_new_chat {
        let row = adw::ActionRow::builder()
            .title("New chat")
            .activatable(true)
            .build();
        row.set_widget_name("iris-keyboard-chat-new");
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&row);
        body.append(&list);
    }
    // Grouping must not introduce extra Tab stops.
    for row in [&first, &second] {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(row);
        body.append(&list);
    }
    let remaining = gtk::ListBox::new();
    remaining.set_selection_mode(gtk::SelectionMode::None);
    for i in 2..140 {
        let row = adw::ActionRow::builder()
            .title(format!("Synthetic chat {i}"))
            .activatable(true)
            .build();
        row.set_widget_name(&format!("iris-keyboard-chat-{i}"));
        remaining.append(&row);
    }
    body.append(&remaining);
    keyboard_list::install(&body);
    (body, first, second)
}

fn settle() {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(150);
    while std::time::Instant::now() < until {
        pump();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn observe_activations(body: &gtk::Box, count: &Rc<Cell<u32>>) {
    let mut section = body.first_child();
    while let Some(list) = section {
        let mut child = list.first_child();
        while let Some(widget) = child {
            if let Some(row) = widget.downcast_ref::<adw::ActionRow>() {
                let count = count.clone();
                row.connect_activated(move |_| count.set(count.get() + 1));
            }
            child = widget.next_sibling();
        }
        section = list.next_sibling();
    }
}

fn main() {
    adw::init().expect("GTK display required");
    message_timeline::run();
    let css = gtk::CssProvider::new();
    css.load_from_string(&format!(
        "@define-color iris_accent #702ACE; {}",
        keyboard_list::CSS
    ));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let window = adw::Window::builder()
        .default_width(420)
        .default_height(400)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let search = gtk::SearchEntry::new();
    search.set_widget_name("iris-keyboard-search");
    let after = gtk::Button::with_label("Settings");
    let (body, first, second) = list(false);
    root.append(&search);
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&body)
        .build();
    root.append(&scroll);
    root.append(&after);
    window.set_content(Some(&root));
    window.present();
    settle();
    let activated = Rc::new(Cell::new(0));
    observe_activations(&body, &activated);
    search.grab_focus();
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(first.has_focus(), "Tab reaches the first chat");
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(
        second.has_focus(),
        "Tab reaches the next chat across sections"
    );
    for i in 2..140 {
        assert!(root.child_focus(gtk::DirectionType::TabForward));
        assert_eq!(
            gtk::prelude::RootExt::focus(&window).unwrap().widget_name(),
            format!("iris-keyboard-chat-{i}")
        );
    }
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(
        after.has_focus(),
        "Tab leaves the list after its final chat"
    );
    assert!(root.child_focus(gtk::DirectionType::TabBackward));
    for i in (2..140).rev() {
        assert_eq!(
            gtk::prelude::RootExt::focus(&window).unwrap().widget_name(),
            format!("iris-keyboard-chat-{i}")
        );
        assert!(root.child_focus(gtk::DirectionType::TabBackward));
    }
    assert!(second.has_focus());
    assert!(root.child_focus(gtk::DirectionType::TabBackward));
    assert!(first.has_focus(), "Shift-Tab crosses section boundaries");
    assert!(root.child_focus(gtk::DirectionType::TabBackward));
    assert!(
        gtk::prelude::RootExt::focus(&window)
            .unwrap()
            .is_ancestor(&search),
        "Shift-Tab exits to search"
    );
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(first.has_focus());
    assert_eq!(activated.get(), 0, "Tab navigation does not open a chat");
    let adj = scroll.vadjustment();
    adj.set_value(0.0);
    for _ in 0..80 {
        assert!(key(&body, gtk::gdk::Key::Down));
        pump();
    }
    assert!(
        adj.value() > 2000.0 && first.has_focus(),
        "Long Down keeps offscreen row focus"
    );
    assert_eq!(
        activated.get(),
        0,
        "Scrolling must not change the open chat"
    );
    for _ in 0..80 {
        assert!(key(&body, gtk::gdk::Key::Up));
        pump();
    }
    assert!(adj.value() < 1.0 && first.has_focus());
    for _ in 0..80 {
        assert!(key(&body, gtk::gdk::Key::Down));
        pump();
    }
    assert!(
        adj.value() > 2000.0 && first.has_focus(),
        "Offscreen focus continues receiving arrows"
    );
    for activate_key in [gtk::gdk::Key::Return, gtk::gdk::Key::space] {
        assert!(!key(&body, activate_key), "Native activation retained");
        adw::prelude::ActionRowExt::activate(&first);
    }
    assert_eq!(activated.get(), 2);
    let offset = adj.value();
    second.grab_focus();
    adj.set_value(offset);
    settle();
    let saved = keyboard_list::FocusBookmark::capture(root.upcast_ref()).unwrap();
    let weak = body.downgrade();
    scroll.set_child(gtk::Widget::NONE);
    drop(first);
    drop(second);
    drop(body);
    let (body, first, second) = list(true);
    observe_activations(&body, &activated);
    scroll.set_child(Some(&body));
    saved.restore(root.upcast_ref());
    settle();
    assert!(
        second.has_focus(),
        "Refresh preserves the offscreen focused identity after a preceding chat is inserted"
    );
    assert!(
        (adj.value() - offset).abs() < 2.0,
        "Refresh preserves viewport after focus restoration"
    );
    assert!(
        weak.upgrade().is_none(),
        "Controllers release destroyed lists"
    );
    assert!(key(&body, gtk::gdk::Key::Home));
    pump();
    assert!(adj.value() < 1.0 && second.has_focus());
    assert!(key(&body, gtk::gdk::Key::End));
    pump();
    assert!((adj.value() - (adj.upper() - adj.page_size())).abs() < 2.0 && second.has_focus());
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert_eq!(
        gtk::prelude::RootExt::focus(&window).unwrap().widget_name(),
        "iris-keyboard-chat-2",
        "Tab continues from the restored chat identity"
    );
    assert!(root.child_focus(gtk::DirectionType::TabBackward));
    assert!(second.has_focus(), "Shift-Tab returns to the restored chat");
    assert_eq!(
        activated.get(),
        2,
        "Restoration and subsequent Tab do not activate chats"
    );

    search.set_text("some text");
    search.grab_focus();
    search.set_position(4);
    let saved = keyboard_list::FocusBookmark::capture(root.upcast_ref()).unwrap();
    saved.restore(root.upcast_ref());
    assert_eq!(search.position(), 4, "Search cursor survives refresh");
    let input = gtk::TextView::new();
    input.set_accepts_tab(true);
    input.set_widget_name("iris-chat-composer");
    root.append(&input);
    input.grab_focus();
    assert!(
        !key(&body, gtk::gdk::Key::Up),
        "List navigation leaves composer editing alone"
    );
    assert!(
        keyboard_list::FocusBookmark::capture(body.upcast_ref()).is_none(),
        "Background list updates do not steal composer focus"
    );
    input.buffer().set_text("draft");
    assert!(keyboard_list::focus_list(root.upcast_ref()));
    assert_eq!(
        gtk::prelude::RootExt::focus(&window).unwrap().widget_name(),
        "iris-keyboard-chat-new"
    );
    assert!(keyboard_list::focus_composer(root.upcast_ref()));
    assert!(input.has_focus() && input.accepts_tab());
    assert_eq!(
        input.buffer().text(
            &input.buffer().start_iter(),
            &input.buffer().end_iter(),
            false
        ),
        "draft"
    );
    first.grab_focus();
    adj.set_value(0.0);
    window.set_focus_visible(true);
    settle();
    if let Some(path) = std::env::var_os("IRIS_KEYBOARD_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let node = loop {
            pump();
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            if let Some(node) = snapshot.to_node() {
                break node;
            }
            assert!(std::time::Instant::now() < deadline, "Snapshot timed out");
        };
        window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(path)
            .unwrap();
    }
    window.close();
    pump();
    println!("PASS: GTK Tab/Shift-Tab through 140 chats, viewport-only arrows, identity/viewport after insertion, native activation, composer isolation");
}
