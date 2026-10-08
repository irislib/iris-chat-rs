use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, glib};

const MIN_LEVEL: i32 = -3;
const MAX_LEVEL: i32 = 4;
const LABELS: [&str; 8] = [
    "Smallest",
    "Smaller",
    "Small",
    "Default",
    "Large",
    "Larger",
    "Very large",
    "Largest",
];

thread_local! {
    static MODELS: RefCell<HashMap<PathBuf, Weak<TextSize>>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

struct TextSize {
    level: gtk::Adjustment,
    provider: gtk::CssProvider,
    display: gdk::Display,
    root_class: String,
}

impl TextSize {
    fn get(data_dir: &Path, display: &gdk::Display) -> Rc<Self> {
        MODELS.with(|models| {
            let mut models = models.borrow_mut();
            models.retain(|_, model| model.strong_count() > 0);
            if let Some(model) = models.get(data_dir).and_then(Weak::upgrade) {
                return model;
            }
            let model = Self::new(data_dir, display);
            models.insert(data_dir.to_owned(), Rc::downgrade(&model));
            model
        })
    }

    fn new(data_dir: &Path, display: &gdk::Display) -> Rc<Self> {
        let path = data_dir.join("desktop-zoom.txt");
        let saved = std::fs::read_to_string(&path)
            .ok()
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(0)
            .clamp(MIN_LEVEL, MAX_LEVEL);
        let id = NEXT_ID.with(|next| {
            let id = next.get();
            next.set(id + 1);
            id
        });
        let model = Rc::new(Self {
            level: gtk::Adjustment::new(
                f64::from(saved),
                f64::from(MIN_LEVEL),
                f64::from(MAX_LEVEL),
                1.0,
                1.0,
                0.0,
            ),
            provider: gtk::CssProvider::new(),
            display: display.clone(),
            root_class: format!("iris-text-size-{id}"),
        });
        model.apply();
        gtk::style_context_add_provider_for_display(
            display,
            &model.provider,
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
        let weak = Rc::downgrade(&model);
        model.level.connect_value_changed(move |level| {
            if let Some(model) = weak.upgrade() {
                model.apply();
                let _ = sender.send(level.value() as i32);
            }
        });
        model
    }

    fn apply(&self) {
        // Apply relative scaling exactly once at the window. Shared color
        // classes such as iris-root also occur on nested panels and toolbars.
        self.provider.load_from_string(&format!(
            ".{} {{ font-size: {}%; }}",
            self.root_class,
            100.0 * 1.2_f64.powi(self.level.value() as i32)
        ));
    }
}

impl Drop for TextSize {
    fn drop(&mut self) {
        gtk::style_context_remove_provider_for_display(&self.display, &self.provider);
    }
}

pub fn install(window: &impl IsA<gtk::Widget>, data_dir: &Path) {
    let model = TextSize::get(data_dir, &window.display());
    window.add_css_class(&model.root_class);
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("iris-text-size-shortcuts"));
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            || modifiers.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK)
        {
            return glib::Propagation::Proceed;
        }
        let current = model.level.value() as i32;
        let next = match key {
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => (current + 1).min(MAX_LEVEL),
            gdk::Key::minus | gdk::Key::KP_Subtract => (current - 1).max(MIN_LEVEL),
            gdk::Key::_0 | gdk::Key::KP_0 => 0,
            _ => return glib::Propagation::Proceed,
        };
        model.level.set_value(f64::from(next));
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

pub fn settings_row(data_dir: &Path) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title("Text size")
        .use_subtitle(true)
        .model(&gtk::StringList::new(&LABELS))
        .build();
    row.set_widget_name("textSizeSetting");
    let model = TextSize::get(data_dir, &row.display());
    row.set_selected((model.level.value() as i32 - MIN_LEVEL) as u32);
    let for_selection = model.clone();
    row.connect_selected_notify(move |row| {
        if row.selected() < LABELS.len() as u32 {
            for_selection
                .level
                .set_value(f64::from(row.selected() as i32 + MIN_LEVEL));
        }
    });
    let weak = row.downgrade();
    let handler = model.level.connect_value_changed(move |level| {
        if let Some(row) = weak.upgrade() {
            row.set_selected((level.value() as i32 - MIN_LEVEL) as u32);
        }
    });
    let handler = RefCell::new(Some(handler));
    row.connect_destroy(move |_| {
        if let Some(handler) = handler.borrow_mut().take() {
            model.level.disconnect(handler);
        }
    });
    row
}

#[cfg(feature = "ui-tests")]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
