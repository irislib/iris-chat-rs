use super::*;

pub fn run() {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
    list.set_margin_top(24);
    list.set_margin_start(24);
    list.set_margin_end(24);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    let window = gtk::Window::builder()
        .default_width(420)
        .default_height(300)
        .child(&scroll)
        .build();
    let selection = MessageSelection::new(&scroll);
    let labels: Vec<_> = [
        "First message",
        "Middle 👩🏽‍💻 message\nsecond line",
        "Last message",
    ]
    .iter()
    .map(|text| {
        let label = gtk::Label::new(Some(text));
        label.set_selectable(true);
        label.set_xalign(0.0);
        label.add_css_class(BODY_CLASS);
        list.append(&label);
        label
    })
    .collect();
    // A timestamp is deliberately selectable but is not a message body.
    let metadata = gtk::Label::new(Some("12:34"));
    metadata.set_selectable(true);
    list.append(&metadata);
    let rows: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| (i.to_string(), label.clone().upcast()))
        .collect();
    selection.update(&rows);
    window.present();
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
    {
        let mut state = selection.0.borrow_mut();
        state.anchor = Some(Position { row: 0, byte: 6 });
        state.end = Some(Position { row: 2, byte: 4 });
        state.paint();
        assert_eq!(
            state.selected_text(),
            "message\nMiddle 👩🏽‍💻 message\nsecond line\nLast"
        );
        assert!(labels.iter().all(|label| label.attributes().is_some()));
        assert!(
            metadata.attributes().is_none(),
            "Metadata is excluded from highlighting"
        );
        state.anchor = Some(Position { row: 2, byte: 4 });
        state.end = Some(Position { row: 0, byte: 6 });
        assert_eq!(
            state.selected_text(),
            "message\nMiddle 👩🏽‍💻 message\nsecond line\nLast"
        );
    }
    if let Some(directory) = std::env::var_os("IRIS_SELECTION_EVIDENCE_DIR") {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let node = loop {
            while context.pending() {
                context.iteration(false);
            }
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                f64::from(window.width()),
                f64::from(window.height()),
            );
            if let Some(node) = snapshot.to_node() {
                break node;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Selection screenshot timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(std::path::Path::new(&directory).join("gtk-selection.png"))
            .unwrap();
    }
    let older = gtk::Label::new(Some("Earlier"));
    older.add_css_class(BODY_CLASS);
    list.prepend(&older);
    let mut updated = vec![("older".into(), older.upcast())];
    updated.extend(rows.clone());
    selection.update(&updated);
    assert_eq!(
        selection.0.borrow().selected_text(),
        "message\nMiddle 👩🏽‍💻 message\nsecond line\nLast"
    );
    labels[1].set_text("Edited");
    selection.update(&updated);
    assert!(
        selection.0.borrow().selected_text().is_empty(),
        "Edits clear stale selection"
    );
    assert!(
        labels.iter().all(|label| label.attributes().is_none()),
        "Clearing restores original styling"
    );
    {
        let mut state = selection.0.borrow_mut();
        state.anchor = Some(Position { row: 1, byte: 0 });
        state.end = Some(Position { row: 3, byte: 4 });
        state.paint();
    }
    selection.update(&updated[..3]);
    assert!(
        selection.0.borrow().selected_text().is_empty(),
        "Removing an endpoint clears selection"
    );
    window.close();
    println!("PASS: GTK message text selection, reverse order, Unicode, metadata exclusion, prepends, edits and deletion");
}
