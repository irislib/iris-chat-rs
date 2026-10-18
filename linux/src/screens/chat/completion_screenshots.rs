use gtk::prelude::*;
use std::time::{Duration, Instant};

pub(super) fn save(window: &gtk::Window, name: &str) {
    save_widget(window.upcast_ref(), name);
}

pub(super) fn save_widget(widget: &gtk::Widget, name: &str) {
    let Some(directory) = std::env::var_os("IRIS_COMPLETION_SCREENSHOT_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let context = gtk::glib::MainContext::default();
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let deadline = Instant::now() + Duration::from_secs(5);
    let node = loop {
        while context.pending() {
            context.iteration(false);
        }
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
        if let Some(node) = snapshot.to_node() {
            break node;
        }
        assert!(Instant::now() < deadline, "Completion did not render");
        std::thread::sleep(Duration::from_millis(2));
    };
    widget
        .native()
        .unwrap()
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png(directory.join(format!("{name}.png")))
        .unwrap();
}
