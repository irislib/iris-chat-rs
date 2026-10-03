use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

// Retain the viewport and unchanged rows. A visible row, rather than a guessed
// height, anchors prepends and later attachment/text size changes.
#[derive(Clone)]
pub struct MessageTimeline {
    pub scroll: gtk::ScrolledWindow,
    list: gtk::Box,
    state: Rc<RefCell<Viewport>>,
    adjusting: Rc<Cell<bool>>,
}
#[derive(Default)]
struct Viewport {
    following: bool,
    rows: Vec<(String, gtk::Widget)>,
    anchor: Option<(String, f64)>,
}
impl MessageTimeline {
    pub fn new(load_older: impl Fn() + 'static) -> Self {
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_valign(gtk::Align::End);
        list.set_margin_top(8);
        list.set_margin_bottom(8);
        list.set_margin_start(10);
        list.set_margin_end(10);
        scroll.set_child(Some(&list));
        let timeline = Self {
            scroll,
            list,
            state: Rc::new(RefCell::new(Viewport {
                following: true,
                ..Default::default()
            })),
            adjusting: Rc::new(Cell::new(false)),
        };
        let weak = timeline.scroll.downgrade();
        let state = Rc::downgrade(&timeline.state);
        let adjusting = timeline.adjusting.clone();
        timeline
            .scroll
            .vadjustment()
            .connect_value_changed(move |adj| {
                if adjusting.get() {
                    return;
                }
                let Some(scroll) = weak.upgrade() else {
                    return;
                };
                let Some(state) = state.upgrade() else {
                    return;
                };
                {
                    let mut state = state.borrow_mut();
                    state.following = adj.value() >= bottom(adj) - 24.0;
                    state.anchor = visible_anchor(&scroll, &state.rows);
                }
                if adj.value() < 120.0 && bottom(adj) > 0.0 {
                    load_older();
                }
            });
        let weak = timeline.scroll.downgrade();
        let state = Rc::downgrade(&timeline.state);
        let adjusting = timeline.adjusting.clone();
        timeline.scroll.vadjustment().connect_changed(move |_| {
            let weak = weak.clone();
            let state = state.clone();
            let adjusting = adjusting.clone();
            // GTK emits changed during allocation; changing the value inside
            // that allocation can leave the viewport using its old transform.
            gtk::glib::idle_add_local_once(move || {
                if let (Some(scroll), Some(state)) = (weak.upgrade(), state.upgrade()) {
                    restore(&scroll, &state.borrow(), &adjusting);
                }
            });
        });
        timeline
    }

    pub fn follow_latest(&self) {
        self.state.borrow_mut().following = true;
        restore(&self.scroll, &self.state.borrow(), &self.adjusting);
    }

    pub fn update(&self, rows: Vec<(String, gtk::Widget)>) {
        let mut state = self.state.borrow_mut();
        if state.rows == rows {
            return;
        }
        if !state.following {
            state.anchor = visible_anchor(&self.scroll, &state.rows);
        }
        self.adjusting.set(true);
        let wanted: HashSet<_> = rows.iter().map(|(_, widget)| widget).collect();
        for (_, widget) in &state.rows {
            if !wanted.contains(widget) {
                self.list.remove(widget);
            }
        }
        let mut previous: Option<&gtk::Widget> = None;
        for (_, widget) in &rows {
            if widget.parent().is_none() {
                self.list.insert_child_after(widget, previous);
            } else if widget.prev_sibling().as_ref() != previous {
                self.list.reorder_child_after(widget, previous);
            }
            previous = Some(widget);
        }
        state.rows = rows;
        drop(state);
        self.adjusting.set(false);
        // Extent changes restore after allocation. This also covers an update
        // whose different row heights happen to leave the total extent equal.
        let weak = self.scroll.downgrade();
        let state = Rc::downgrade(&self.state);
        let adjusting = self.adjusting.clone();
        self.scroll.add_tick_callback(move |_, clock| {
            let weak = weak.clone();
            let state = state.clone();
            let adjusting = adjusting.clone();
            let handler = Rc::new(RefCell::new(None));
            let once = handler.clone();
            *handler.borrow_mut() = Some(clock.connect_after_paint(move |clock| {
                if let (Some(scroll), Some(state)) = (weak.upgrade(), state.upgrade()) {
                    restore(&scroll, &state.borrow(), &adjusting);
                }
                if let Some(id) = once.borrow_mut().take() {
                    clock.disconnect(id);
                }
            }));
            gtk::glib::ControlFlow::Break
        });
    }
}
fn bottom(adj: &gtk::Adjustment) -> f64 {
    (adj.upper() - adj.page_size()).max(adj.lower())
}
fn visible_anchor(
    scroll: &gtk::ScrolledWindow,
    rows: &[(String, gtk::Widget)],
) -> Option<(String, f64)> {
    rows.iter().find_map(|(id, widget)| {
        let bounds = widget.compute_bounds(&widget.parent()?)?;
        let y = f64::from(bounds.y()) - scroll.vadjustment().value();
        (y + f64::from(bounds.height()) > 0.0).then(|| (id.clone(), y))
    })
}
fn restore(scroll: &gtk::ScrolledWindow, state: &Viewport, adjusting: &Cell<bool>) {
    if adjusting.replace(true) {
        return;
    }
    let adj = scroll.vadjustment();
    if state.following {
        adj.set_value(bottom(&adj));
    } else if let Some((id, y)) = &state.anchor {
        if let Some((_, widget)) = state.rows.iter().find(|(key, _)| key == id) {
            if let Some(bounds) = widget
                .parent()
                .and_then(|parent| widget.compute_bounds(&parent))
            {
                adj.set_value(f64::from(bounds.y()) - y);
            }
        }
    }
    adjusting.set(false);
}
