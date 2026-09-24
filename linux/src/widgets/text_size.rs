use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

pub fn install(window: &impl IsA<gtk::Widget>, data_dir: &Path) {
    let path = data_dir.join("desktop-zoom.txt");
    let saved = std::fs::read_to_string(&path)
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(0)
        .clamp(-3, 4);
    let level = Rc::new(Cell::new(saved));
    let provider = gtk::CssProvider::new();
    // Relative size preserves the user's GTK/system font preference.
    let apply = |provider: &gtk::CssProvider, level: i32| {
        provider.load_from_string(&format!(
            ".iris-root {{ font-size: {}%; }}",
            100.0 * 1.2_f64.powi(level)
        ));
    };
    apply(&provider, saved);
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(window),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let (sender, receiver) = std::sync::mpsc::channel::<i32>();
    std::thread::spawn(move || {
        while let Ok(mut value) = receiver.recv() {
            while let Ok(latest) = receiver.try_recv() {
                value = latest;
            }
            let _ = std::fs::write(&path, value.to_string());
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            || modifiers.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK)
        {
            return glib::Propagation::Proceed;
        }
        let next = match key {
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => (level.get() + 1).min(4),
            gdk::Key::minus | gdk::Key::KP_Subtract => (level.get() - 1).max(-3),
            gdk::Key::_0 | gdk::Key::KP_0 => 0,
            _ => return glib::Propagation::Proceed,
        };
        level.set(next);
        apply(&provider, next);
        let _ = sender.send(next);
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}
