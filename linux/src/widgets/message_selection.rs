use adw::prelude::*;
use gtk::{gdk, glib, pango};
use std::cell::RefCell;
use std::rc::Rc;

pub const BODY_CLASS: &str = "message-selectable-body";

#[derive(Clone)]
struct Body {
    label: gtk::Label,
    text: String,
    attributes: Option<pango::AttrList>,
}

#[derive(Clone, Copy)]
struct Position {
    row: usize,
    byte: usize,
}

#[derive(Default)]
struct State {
    bodies: Vec<Body>,
    anchor: Option<Position>,
    end: Option<Position>,
    dragging: bool,
    point: Option<(f64, f64)>,
    timer: Option<glib::SourceId>,
}

/// GTK labels own individual native selections. Keep that behavior for ordinary
/// clicks/word selection, taking over only when a drag crosses message bodies.
#[derive(Clone)]
pub struct MessageSelection(Rc<RefCell<State>>);

impl MessageSelection {
    pub fn new(scroll: &gtk::ScrolledWindow) -> Self {
        let selection = Self(Rc::new(RefCell::new(State::default())));
        // GtkLabel claims its drag gesture on press, denying parent gestures.
        // Observe raw capture events so its normal click/word/link behavior can
        // continue until the pointer actually enters another message.
        let pointer = gtk::EventControllerLegacy::new();
        pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state = selection.clone();
        let weak = scroll.downgrade();
        pointer.connect_event(move |_, event| {
            let Some(scroll) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let point = event_point(event, &scroll);
            let left = event
                .downcast_ref::<gdk::ButtonEvent>()
                .is_some_and(|e| e.button() == 1);
            match event.event_type() {
                gdk::EventType::ButtonPress if left => {
                    state.clear();
                    if let Some((x, y)) = point {
                        let mut hit = scroll.pick(x, y, gtk::PickFlags::DEFAULT);
                        while let Some(widget) = hit {
                            if let Some(label) = widget
                                .downcast_ref::<gtk::Label>()
                                .filter(|l| l.has_css_class(BODY_CLASS))
                            {
                                let mut state = state.0.borrow_mut();
                                state.anchor = state
                                    .bodies
                                    .iter()
                                    .position(|body| body.label == *label)
                                    .map(|row| Position {
                                        row,
                                        byte: insertion(label, &scroll, x, y),
                                    });
                                break;
                            }
                            hit = widget.parent();
                        }
                    }
                }
                gdk::EventType::MotionNotify
                    if event
                        .modifier_state()
                        .contains(gdk::ModifierType::BUTTON1_MASK) =>
                {
                    let Some(point) = point else {
                        return glib::Propagation::Proceed;
                    };
                    let crossing = {
                        let state = state.0.borrow();
                        state
                            .anchor
                            .zip(state.position(&scroll, point))
                            .is_some_and(|(a, b)| {
                                a.row != b.row
                                    || point.1 < 24.0
                                    || point.1 > f64::from(scroll.height()) - 24.0
                            })
                    };
                    if crossing && !state.0.borrow().dragging {
                        let origin = {
                            let s = state.0.borrow();
                            s.anchor.map(|a| s.bodies[a.row].label.clone())
                        };
                        if let Some(label) = origin {
                            let controllers = label.observe_controllers();
                            for i in 0..controllers.n_items() {
                                if let Some(controller) = controllers
                                    .item(i)
                                    .and_then(|v| v.downcast::<gtk::EventController>().ok())
                                {
                                    controller.reset();
                                }
                            }
                        }
                        state.start_drag(&scroll);
                    }
                    let mut state = state.0.borrow_mut();
                    if state.dragging {
                        state.point = Some(point);
                        state.extend(&scroll, point);
                        return glib::Propagation::Stop;
                    }
                }
                gdk::EventType::ButtonRelease if left => {
                    let mut state = state.0.borrow_mut();
                    let dragging = state.dragging;
                    if dragging {
                        if let Some(point) = point {
                            state.extend(&scroll, point);
                        }
                    }
                    state.stop_drag();
                    if dragging {
                        return glib::Propagation::Stop;
                    }
                    state.anchor = None;
                }
                gdk::EventType::GrabBroken => state.0.borrow_mut().stop_drag(),
                _ => {}
            }
            glib::Propagation::Proceed
        });
        scroll.add_controller(pointer);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state = selection.clone();
        let weak = scroll.downgrade();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gdk::Key::Escape {
                let had_selection = !state.0.borrow().selected_text().is_empty();
                state.clear();
                return if had_selection {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                };
            }
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::c | gdk::Key::C)
            {
                let text = state.0.borrow().selected_text();
                if !text.is_empty() {
                    if let Some(scroll) = weak.upgrade() {
                        scroll.clipboard().set_text(&text);
                    }
                    return glib::Propagation::Stop;
                }
            }
            if matches!(
                key,
                gdk::Key::Left
                    | gdk::Key::Right
                    | gdk::Key::Up
                    | gdk::Key::Down
                    | gdk::Key::Home
                    | gdk::Key::End
            ) || (key == gdk::Key::a && modifiers.contains(gdk::ModifierType::CONTROL_MASK))
            {
                state.clear();
            }
            glib::Propagation::Proceed
        });
        scroll.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let state = selection.clone();
        focus.connect_leave(move |_| state.clear());
        scroll.add_controller(focus);
        let state = selection.clone();
        scroll.connect_unmap(move |_| state.clear());

        let context = gtk::GestureClick::new();
        context.set_button(3);
        context.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state = selection.clone();
        let weak = scroll.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            if state.0.borrow().selected_text().is_empty() {
                return;
            }
            let Some(scroll) = weak.upgrade() else { return };
            let menu = gtk::Popover::new();
            let copy = gtk::Button::with_label("Copy");
            copy.add_css_class("flat");
            // Snapshot only for the lifetime of this menu; never put private
            // message text on the clipboard without an explicit copy action.
            let text = state.0.borrow().selected_text();
            let weak_menu = menu.downgrade();
            copy.connect_clicked(move |button| {
                button.clipboard().set_text(&text);
                if let Some(menu) = weak_menu.upgrade() {
                    menu.popdown();
                }
            });
            menu.set_child(Some(&copy));
            menu.set_parent(&scroll);
            menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu.connect_closed(|menu| menu.unparent());
            menu.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        scroll.add_controller(context);
        selection
    }

    fn start_drag(&self, scroll: &gtk::ScrolledWindow) {
        self.0.borrow_mut().dragging = true;
        let weak_state = Rc::downgrade(&self.0);
        let weak_scroll = scroll.downgrade();
        self.0.borrow_mut().timer = Some(glib::timeout_add_local(
            std::time::Duration::from_millis(33),
            move || {
                let (Some(state), Some(scroll)) = (weak_state.upgrade(), weak_scroll.upgrade())
                else {
                    return glib::ControlFlow::Break;
                };
                let Some((x, y)) = state.borrow().point else {
                    return glib::ControlFlow::Continue;
                };
                let delta = if y < 24.0 {
                    -12.0
                } else if y > f64::from(scroll.height()) - 24.0 {
                    12.0
                } else {
                    0.0
                };
                if delta != 0.0 {
                    let adj = scroll.vadjustment();
                    adj.set_value((adj.value() + delta).clamp(
                        adj.lower(),
                        (adj.upper() - adj.page_size()).max(adj.lower()),
                    ));
                    state.borrow_mut().extend(&scroll, (x, y));
                }
                glib::ControlFlow::Continue
            },
        ));
    }

    pub fn update(&self, rows: &[(String, gtk::Widget)]) {
        let mut labels = Vec::new();
        for (_, row) in rows {
            collect(row, &mut labels);
        }
        let mut state = self.0.borrow_mut();
        if labels
            .iter()
            .eq(state.bodies.iter().map(|body| &body.label))
            && state
                .bodies
                .iter()
                .all(|body| body.label.text().as_str() == body.text)
        {
            return;
        }
        // Preserve endpoint widgets across prepends/appends. Replaced/edited or
        // removed selected bodies invalidate the range instead of copying stale text.
        let endpoints = state.anchor.zip(state.end).and_then(|(a, b)| {
            let selected = &state.bodies[a.row.min(b.row)..=a.row.max(b.row)];
            if !selected
                .iter()
                .all(|body| labels.contains(&body.label) && body.label.text().as_str() == body.text)
            {
                return None;
            }
            Some((
                labels
                    .iter()
                    .position(|label| label == &state.bodies[a.row].label)?,
                a.byte,
                labels
                    .iter()
                    .position(|label| label == &state.bodies[b.row].label)?,
                b.byte,
            ))
        });
        if endpoints.is_none() {
            state.clear();
        } else {
            for body in &state.bodies {
                body.label.set_attributes(body.attributes.as_ref());
            }
        }
        state.bodies = labels
            .into_iter()
            .map(|label| Body {
                text: label.text().to_string(),
                attributes: label.attributes(),
                label,
            })
            .collect();
        if let Some((a, start, b, end)) = endpoints {
            state.anchor = Some(Position {
                row: a,
                byte: start,
            });
            state.end = Some(Position { row: b, byte: end });
            state.paint();
        }
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

impl State {
    fn stop_drag(&mut self) {
        self.dragging = false;
        self.point = None;
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
    fn clear(&mut self) {
        self.stop_drag();
        self.anchor = None;
        self.end = None;
        for body in &self.bodies {
            body.label.set_attributes(body.attributes.as_ref());
        }
    }
    fn range(&self, row: usize) -> Option<std::ops::Range<usize>> {
        let (a, b) = self.anchor.zip(self.end)?;
        let (a, b) = if (a.row, a.byte) <= (b.row, b.byte) {
            (a, b)
        } else {
            (b, a)
        };
        if row < a.row || row > b.row {
            return None;
        }
        let start = if row == a.row { a.byte } else { 0 };
        let end = if row == b.row {
            b.byte
        } else {
            self.bodies[row].text.len()
        };
        (start < end).then_some(start..end)
    }
    fn selected_text(&self) -> String {
        self.bodies
            .iter()
            .enumerate()
            .filter_map(|(row, body)| body.text.get(self.range(row)?).map(str::to_owned))
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn position(&self, scroll: &gtk::ScrolledWindow, (x, y): (f64, f64)) -> Option<Position> {
        let (row, body) = self.bodies.iter().enumerate().min_by(|(_, a), (_, b)| {
            let distance = |body: &Body| {
                body.label
                    .compute_bounds(scroll)
                    .map(|rect| {
                        (f64::from(rect.y()) - y)
                            .max(y - f64::from(rect.y() + rect.height()))
                            .max(0.0)
                    })
                    .unwrap_or(f64::MAX)
            };
            distance(a).total_cmp(&distance(b))
        })?;
        Some(Position {
            row,
            byte: insertion(&body.label, scroll, x, y),
        })
    }
    fn extend(&mut self, scroll: &gtk::ScrolledWindow, point: (f64, f64)) {
        self.end = self.position(scroll, point);
        self.paint();
    }
    fn paint(&self) {
        for (row, body) in self.bodies.iter().enumerate() {
            body.label.select_region(0, 0);
            let Some(range) = self.range(row) else {
                body.label.set_attributes(body.attributes.as_ref());
                continue;
            };
            let attrs = body
                .attributes
                .as_ref()
                .and_then(|attrs| attrs.copy())
                .unwrap_or_default();
            let style = body.label.style_context();
            let bg = style
                .lookup_color("accent_bg_color")
                .unwrap_or(gdk::RGBA::new(0.2, 0.4, 0.8, 1.0));
            let fg = style
                .lookup_color("accent_fg_color")
                .unwrap_or(gdk::RGBA::WHITE);
            for mut attr in [
                pango::AttrColor::new_background(
                    (bg.red() * 65535.0) as u16,
                    (bg.green() * 65535.0) as u16,
                    (bg.blue() * 65535.0) as u16,
                ),
                pango::AttrColor::new_foreground(
                    (fg.red() * 65535.0) as u16,
                    (fg.green() * 65535.0) as u16,
                    (fg.blue() * 65535.0) as u16,
                ),
            ] {
                attr.set_start_index(range.start as u32);
                attr.set_end_index(range.end as u32);
                attrs.insert(attr);
            }
            body.label.set_attributes(Some(&attrs));
        }
    }
}

fn collect(widget: &gtk::Widget, labels: &mut Vec<gtk::Label>) {
    if widget.has_css_class(BODY_CLASS) {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
            labels.push(label);
        }
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        collect(&widget, labels);
    }
}

fn insertion(label: &gtk::Label, scroll: &gtk::ScrolledWindow, x: f64, y: f64) -> usize {
    let Some(point) = scroll.compute_point(label, &gtk::graphene::Point::new(x as f32, y as f32))
    else {
        return 0;
    };
    let text = label.text();
    if point.y() < 0.0 {
        return 0;
    }
    if point.y() > label.height() as f32 {
        return text.len();
    }
    let (ox, oy) = label.layout_offsets();
    let (_, index, trailing) = label.layout().xy_to_index(
        ((point.x() - ox as f32) * pango::SCALE as f32) as i32,
        ((point.y() - oy as f32) * pango::SCALE as f32) as i32,
    );
    let start = (index.max(0) as usize).min(text.len());
    start
        + text[start..]
            .chars()
            .take(trailing.max(0) as usize)
            .map(char::len_utf8)
            .sum::<usize>()
}

fn event_point(event: &gdk::Event, scroll: &gtk::ScrolledWindow) -> Option<(f64, f64)> {
    let (x, y) = event.position()?;
    let native = scroll.native()?;
    let (tx, ty) = native.surface_transform();
    let point = native.compute_point(
        scroll,
        &gtk::graphene::Point::new((x + tx) as f32, (y + ty) as f32),
    )?;
    Some((f64::from(point.x()), f64::from(point.y())))
}

#[cfg(feature = "ui-tests")]
#[path = "message_selection/tests.rs"]
pub mod tests;
