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

fn list() -> (gtk::Box, adw::ActionRow, adw::ActionRow) {
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
    // Grouping must not introduce extra Tab stops or block arrow movement.
    for row in [&first, &second] {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(row);
        body.append(&list);
    }
    keyboard_list::install(&body);
    (body, first, second)
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
    let (body, first, second) = list();
    root.append(&search);
    root.append(&body);
    root.append(&after);
    window.set_content(Some(&root));
    window.present();
    pump();
    search.grab_focus();
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(first.has_focus(), "Tab reaches the first chat");
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(after.has_focus(), "Tab skips remaining chat rows");
    assert!(root.child_focus(gtk::DirectionType::TabBackward));
    assert!(first.has_focus(), "Shift-Tab returns to the list");
    let activated = Rc::new(Cell::new(0));
    let observed = activated.clone();
    second.connect_activated(move |_| observed.set(observed.get() + 1));
    assert!(key(&body, gtk::gdk::Key::Down));
    assert!(second.has_focus());
    assert_eq!(activated.get(), 0, "Arrows must not open a chat");
    assert!(
        !key(&body, gtk::gdk::Key::Return),
        "Enter remains GTK's native row activation"
    );
    adw::prelude::ActionRowExt::activate(&second);
    assert_eq!(activated.get(), 1);
    assert!(root.child_focus(gtk::DirectionType::TabForward));
    assert!(after.has_focus(), "One Tab exits all chat sections");
    assert!(root.child_focus(gtk::DirectionType::TabBackward));
    assert!(second.has_focus(), "Shift-Tab returns to the same row");
    assert!(key(&body, gtk::gdk::Key::Home));
    assert!(first.has_focus());
    key(&body, gtk::gdk::Key::End);
    assert!(second.has_focus());
    let saved = keyboard_list::FocusBookmark::capture(root.upcast_ref()).unwrap();
    let weak = body.downgrade();
    root.remove(&body);
    drop(first);
    drop(second);
    drop(body);
    let (body, first, second) = list();
    root.insert_child_after(&body, Some(&search));
    saved.restore(root.upcast_ref());
    assert!(second.has_focus(), "Refresh preserves the focused chat");
    assert!(!first.is_focusable(), "Restore keeps one Tab entry");
    pump();
    assert!(
        weak.upgrade().is_none(),
        "Keyboard controllers must not retain destroyed lists"
    );

    search.set_text("some text");
    search.grab_focus();
    search.set_position(4);
    let saved = keyboard_list::FocusBookmark::capture(root.upcast_ref()).unwrap();
    saved.restore(root.upcast_ref());
    assert_eq!(search.position(), 4, "Search cursor survives refresh");
    let input = gtk::TextView::new();
    input.set_accepts_tab(false);
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
    second.grab_focus();
    window.set_focus_visible(true);
    pump();
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
    println!("PASS: GTK Tab/Shift-Tab, grouped arrows, native activation, refresh, cursor and composer focus");
}
