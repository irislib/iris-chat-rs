use gtk::prelude::*;
use std::rc::Rc;

// Native rows retain their activation/accessibility behavior. A roving focus
// target makes all sidebar sections one Tab stop without trapping the composer.
pub fn install(container: &impl IsA<gtk::Widget>) {
    let rows = rows(container.as_ref());
    let weak_rows = Rc::new(rows.iter().map(|row| row.downgrade()).collect::<Vec<_>>());
    for (index, row) in rows.iter().enumerate() {
        row.set_focusable(index == 0);
        let focus = gtk::EventControllerFocus::new();
        let rows = weak_rows.clone();
        focus.connect_enter(move |controller| {
            if let Some(current) = controller.widget() {
                for row in rows.iter().filter_map(|row| row.upgrade()) {
                    row.set_focusable(row == current);
                }
            }
        });
        row.add_controller(focus);
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let rows = weak_rows
            .iter()
            .filter_map(|row| row.upgrade())
            .collect::<Vec<_>>();
        if modifiers.intersects(
            gtk::gdk::ModifierType::SHIFT_MASK
                | gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::META_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        ) {
            return gtk::glib::Propagation::Proceed;
        }
        let Some(index) = rows.iter().position(|row| row.has_focus()) else {
            return gtk::glib::Propagation::Proceed;
        };
        let next = match key {
            gtk::gdk::Key::Up => index.saturating_sub(1),
            gtk::gdk::Key::Down => (index + 1).min(rows.len() - 1),
            gtk::gdk::Key::Home => 0,
            gtk::gdk::Key::End => rows.len() - 1,
            _ => return gtk::glib::Propagation::Proceed,
        };
        rows[next].set_focusable(true);
        rows[next].grab_focus();
        gtk::glib::Propagation::Stop
    });
    container.add_controller(keys);
}

fn rows(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    if !widget.is_visible() || !widget.is_sensitive() {
        return Vec::new();
    }
    if widget.is::<adw::ActionRow>() || widget.is::<gtk::Button>() {
        if widget.is::<gtk::ListBoxRow>()
            && !widget
                .clone()
                .downcast::<gtk::ListBoxRow>()
                .unwrap()
                .is_activatable()
        {
            return Vec::new();
        }
        return vec![widget.clone()];
    }
    if widget.is::<gtk::ListBoxRow>() || widget.is::<gtk::ListBox>() {
        widget.set_focusable(false);
    }
    let mut result = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.extend(rows(&widget));
        child = widget.next_sibling();
    }
    result
}

// Replacing a list must not send keyboard users back to its first row. Only
// restore focus that belonged to the replaced content, never a header/dialog.
pub struct FocusBookmark {
    name: String,
    position: Option<i32>,
}
impl FocusBookmark {
    pub fn capture(root: &gtk::Widget) -> Option<Self> {
        let mut focused = root.root()?.focus()?;
        if !focused.is_ancestor(root) && focused != *root {
            return None;
        }
        loop {
            let name = focused.widget_name();
            if name.starts_with("iris-keyboard-") {
                return Some(Self {
                    name: name.into(),
                    position: focused
                        .downcast_ref::<gtk::SearchEntry>()
                        .map(|entry| entry.position()),
                });
            }
            if focused == *root {
                return None;
            }
            focused = focused.parent()?;
        }
    }
    pub fn restore(self, root: &gtk::Widget) {
        fn find(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
            if root.widget_name() == name {
                return Some(root.clone());
            }
            let mut child = root.first_child();
            while let Some(widget) = child {
                if let Some(found) = find(&widget, name) {
                    return Some(found);
                }
                child = widget.next_sibling();
            }
            None
        }
        if let Some(widget) = find(root, &self.name) {
            widget.set_focusable(true);
            widget.grab_focus();
            if let (Some(entry), Some(position)) =
                (widget.downcast_ref::<gtk::SearchEntry>(), self.position)
            {
                entry.set_position(position);
            }
        }
    }
}

pub const CSS: &str = r#"
button:focus-visible, row:focus-visible {
    outline: 2px solid @iris_accent;
    outline-offset: -2px;
}
"#;
