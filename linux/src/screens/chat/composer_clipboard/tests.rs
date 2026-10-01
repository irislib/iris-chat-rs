use super::*;
use std::time::{Duration, Instant};

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
        assert!(Instant::now() < deadline, "clipboard action timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn widgets(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = vec![root.clone()];
    let mut child = root.first_child();
    while let Some(widget) = child {
        found.extend(widgets(&widget));
        child = widget.next_sibling();
    }
    found
}

pub fn verify_ui(manager: Rc<AppManager>) {
    let original_state = manager.current_state();
    let mut state = original_state.clone();
    state.busy.sending_message = false;
    state.busy.uploading_attachment = false;
    let chat = state.current_chat.as_mut().unwrap();
    chat.direct_chat_capability = None;
    chat.is_request = false;
    chat.kind = iris_chat_core::ChatKind::Direct;
    chat.chat_id = "clipboard-fixture".into();
    chat.draft = "Unsent caption ".into();
    let chat = chat.clone();
    let apply = |mut state: iris_chat_core::AppState| {
        state.rev = manager.current_state().rev + 1;
        manager.apply_update(iris_chat_core::AppUpdate::FullState(state));
    };
    apply(state.clone());
    let composer = super::super::composer::Composer::new(&chat, &state, &manager);
    let window = gtk::Window::builder()
        .default_width(500)
        .default_height(260)
        .build();
    window.set_child(Some(&composer.root));
    window.present();
    let input = widgets(composer.root.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::TextView>().ok())
        .unwrap();
    let direct = widgets(composer.root.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::CheckButton>().ok())
        .unwrap();
    pump_until(|| input.root().is_some());
    let clipboard = input.clipboard();
    let previous = clipboard.content();
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("Original.png");
    let second = dir.path().join("Notes.pdf");
    std::fs::write(&first, [0, 1, 255, 42]).unwrap();
    std::fs::write(&second, b"original PDF bytes").unwrap();
    let files = gdk::FileList::from_array(&[
        gtk::gio::File::for_path(&first),
        gtk::gio::File::for_path(&second),
    ]);
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    let texture = gdk::MemoryTexture::new(
        2,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels.to_vec()),
        8,
    );
    clipboard
        .set_content(Some(&gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_value(&files.to_value()),
            gdk::ContentProvider::for_value(&texture.to_value()),
            gdk::ContentProvider::for_value(
                &"clipboard text should not replace caption".to_value(),
            ),
        ])))
        .unwrap();
    input.emit_paste_clipboard();
    pump_until(|| manager.staged_attachments(&chat.chat_id).len() == 2);
    assert_eq!(
        manager
            .staged_attachments(&chat.chat_id)
            .iter()
            .map(|a| a.file_path.clone())
            .collect::<Vec<_>>(),
        vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned()
        ]
    );
    assert_eq!(std::fs::read(&first).unwrap(), [0, 1, 255, 42]);
    direct.set_active(true);
    clipboard.set_texture(&texture);
    input.emit_paste_clipboard();
    pump_until(|| manager.staged_attachments(&chat.chat_id).len() == 3);
    let image_path = manager
        .staged_attachments(&chat.chat_id)
        .last()
        .unwrap()
        .file_path
        .clone();
    assert_eq!(
        image::open(&image_path).unwrap().into_rgba8().as_raw(),
        &pixels
    );
    assert!(direct.is_active(), "pasting keeps direct-send selection");
    let text = || {
        let buffer = input.buffer();
        let (start, end) = buffer.bounds();
        buffer.text(&start, &end, true).to_string()
    };
    assert_eq!(text(), "Unsent caption ");
    clipboard.set_text("plain text");
    let buffer = input.buffer();
    buffer.place_cursor(&buffer.end_iter());
    input.emit_paste_clipboard();
    pump_until(|| text() == "Unsent caption plain text");
    let remove = widgets(composer.root.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .filter(|button| button.tooltip_text().as_deref() == Some("Remove attachment"))
        .last()
        .unwrap();
    remove.emit_clicked();
    assert!(!std::path::Path::new(&image_path).exists());
    assert!(first.exists() && second.exists());
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);

    // The real Send path calls take_staged_attachments even for text-only
    // messages. Restaging the same paths keeps this test off the network while
    // verifying that a completed Send cannot receive the earlier paste.
    clipboard.set_texture(&texture);
    input.emit_paste_clipboard();
    for attachment in manager.take_staged_attachments(&chat.chat_id) {
        manager.stage_attachment(&chat.chat_id, attachment);
    }
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(100)));
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);

    clipboard.set_texture(&texture);
    input.emit_paste_clipboard();
    direct.set_active(false);
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(100)));
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);
    direct.set_active(true);

    // Editing the caption is still the same draft, so it must not cancel files.
    clipboard.set_texture(&texture);
    input.emit_paste_clipboard();
    buffer.insert_at_cursor(" while pasting");
    pump_until(|| manager.staged_attachments(&chat.chat_id).len() == 3);
    assert_eq!(text(), "Unsent caption plain text while pasting");
    widgets(composer.root.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .filter(|button| button.tooltip_text().as_deref() == Some("Remove attachment"))
        .last()
        .unwrap()
        .emit_clicked();
    buffer.set_text("Unsent caption plain text");

    let invalid_files = gdk::FileList::from_array(&[
        gtk::gio::File::for_path(&first),
        gtk::gio::File::for_path(dir.path()),
    ]);
    clipboard
        .set_content(Some(&gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_value(&invalid_files.to_value()),
            gdk::ContentProvider::for_value(
                &"file-manager labels must not become the caption".to_value(),
            ),
        ])))
        .unwrap();
    input.emit_paste_clipboard();
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(100)));
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);
    assert_eq!(text(), "Unsent caption plain text");

    let url = "https://example.com/copied-link";
    let links = gdk::FileList::from_array(&[gtk::gio::File::for_uri(url)]);
    clipboard
        .set_content(Some(&gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_value(&links.to_value()),
            gdk::ContentProvider::for_value(&url.to_value()),
        ])))
        .unwrap();
    buffer.place_cursor(&buffer.end_iter());
    input.emit_paste_clipboard();
    pump_until(|| text() == format!("Unsent caption plain text{url}"));
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);
    buffer.set_text("Unsent caption plain text");

    clipboard.set_texture(&texture);
    input.emit_paste_clipboard();
    let mut changed = state.clone();
    changed.current_chat.as_mut().unwrap().chat_id = "another-chat".into();
    apply(changed);
    // Run the queued clipboard read; its captured destination is now stale.
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(100)));
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);
    apply(state.clone());
    state.busy.uploading_attachment = true;
    apply(state.clone());
    input.emit_paste_clipboard();
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);
    state.busy.uploading_attachment = false;
    apply(state);
    assert_eq!(text(), "Unsent caption plain text");
    assert_eq!(
        manager.current_state().current_chat.unwrap().messages,
        chat.messages,
        "paste never sends"
    );
    if let Some(path) = std::env::var_os("IRIS_CLIPBOARD_UI_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
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
            .save_to_png(path)
            .unwrap();
    }
    // A large plain-text paste must remain one editor operation, not dispatch
    // a draft update / trigger layout once for every character or line.
    let large = "A long pasted note, with Unicode 世界.\n".repeat(4096);
    buffer.set_text("");
    let changes = Rc::new(std::cell::Cell::new(0));
    let observed = changes.clone();
    buffer.connect_changed(move |_| observed.set(observed.get() + 1));
    clipboard.set_text(&large);
    input.emit_paste_clipboard();
    pump_until(|| text() == large);
    assert_eq!(changes.get(), 1, "large plain text is a single buffer edit");
    assert_eq!(manager.staged_attachments(&chat.chat_id).len(), 2);
    for file in manager.take_staged_attachments(&chat.chat_id) {
        assert!(std::path::Path::new(&file.file_path).exists());
    }
    clipboard.set_content(previous.as_ref()).unwrap();
    window.close();
    apply(original_state);
    println!("PASS: GTK native paste, multiple files, PNG pixels, direct draft, large text single-edit, cleanup, stale/busy guards and no auto-send");
}
