use gtk::prelude::*;

// Tab visits native chat rows; arrows only scroll the viewport.
pub fn install(container: &impl IsA<gtk::Widget>) {
    let rows = rows(container.as_ref());
    let weak_rows = rows.iter().map(|row| row.downgrade()).collect::<Vec<_>>();
    for row in &rows {
        row.set_focusable(true);
    }
    let weak_container = container.as_ref().downgrade();
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
        if !rows.iter().any(|row| row.has_focus()) {
            return gtk::glib::Propagation::Proceed;
        }
        let Some(scroll) = weak_container.upgrade().and_then(|widget| scroll(&widget)) else {
            return gtk::glib::Propagation::Proceed;
        };
        let adjustment = scroll.vadjustment();
        let bottom = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        let value = match key {
            gtk::gdk::Key::Up => adjustment.value() - 40.0,
            gtk::gdk::Key::Down => adjustment.value() + 40.0,
            gtk::gdk::Key::Home => adjustment.lower(),
            gtk::gdk::Key::End => bottom,
            _ => return gtk::glib::Propagation::Proceed,
        };
        adjustment.set_value(value.clamp(adjustment.lower(), bottom));
        gtk::glib::Propagation::Stop
    });
    container.add_controller(keys);
}

fn scroll(widget: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    let mut parent = widget.parent();
    while let Some(widget) = parent {
        if let Ok(scroll) = widget.clone().downcast::<gtk::ScrolledWindow>() {
            return Some(scroll);
        }
        parent = widget.parent();
    }
    None
}

pub fn focus_list(root: &gtk::Widget, preferred_chat: Option<&str>) -> bool {
    let focused = find(root, "iris-keyboard-chat-list")
        .filter(|list| list.is_mapped())
        .and_then(|list| {
            let rows = rows(&list);
            preferred_chat
                .and_then(|id| {
                    let name = format!("iris-keyboard-chat-{id}");
                    rows.iter().find(|row| row.widget_name() == name)
                })
                .or_else(|| rows.first())
                .cloned()
        })
        .is_some_and(|row| row.grab_focus());
    // An empty search still has a useful destination. Do not leave a pending
    // section-focus request waiting for rows that may never appear.
    focused
        || find(root, "iris-keyboard-search")
            .filter(|entry| entry.is_mapped())
            .is_some_and(|entry| entry.grab_focus())
}

pub fn focus_composer(root: &gtk::Widget) -> bool {
    find(root, "iris-chat-composer")
        .filter(|input| input.is_mapped())
        .is_some_and(|input| input.grab_focus())
}

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
    offset: Option<f64>,
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
                    offset: scroll(&focused).map(|scroll| scroll.vadjustment().value()),
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
        if let Some(widget) = find(root, &self.name) {
            widget.grab_focus();
            if let (Some(offset), Some(scroll)) = (self.offset, scroll(&widget)) {
                // Restore after allocation; focusing an offscreen row must not
                // undo the user's viewport-only arrow scrolling.
                scroll.add_tick_callback(move |scroll, _| {
                    scroll.vadjustment().set_value(offset);
                    gtk::glib::ControlFlow::Break
                });
            }
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
