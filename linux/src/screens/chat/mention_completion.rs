use super::mention_completion_logic::{self as logic, Token};
use crate::widgets::clickable::PointerCursorExt;
use gtk::{glib, prelude::*};
use iris_chat_core::ChatParticipantSnapshot as Entry;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
pub(super) struct MentionCompletion {
    pub root: gtk::Box,
    state: Rc<State>,
}

struct Context {
    text: String,
    caret: i32,
    token: Token,
}
struct State {
    input: glib::WeakRef<gtk::TextView>,
    root: glib::WeakRef<gtk::Box>,
    catalog: RefCell<Vec<Entry>>,
    inserted: Box<dyn Fn(i32, String, String)>,
    context: RefCell<Option<Context>>,
    results: RefCell<Vec<Entry>>,
    selected: Cell<usize>,
    preedit: Cell<bool>,
    applying: Cell<bool>,
    undoing: Cell<bool>,
    generation: Cell<u64>,
}

impl MentionCompletion {
    pub fn new(input: &gtk::TextView, inserted: impl Fn(i32, String, String) + 'static) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("chatMentionSuggestions");
        root.add_css_class("card");
        root.set_visible(false);
        let state = Rc::new(State {
            input: input.downgrade(),
            root: root.downgrade(),
            catalog: RefCell::new(vec![]),
            inserted: Box::new(inserted),
            context: RefCell::new(None),
            results: RefCell::new(vec![]),
            selected: Cell::new(0),
            preedit: Cell::new(false),
            applying: Cell::new(false),
            undoing: Cell::new(false),
            generation: Cell::new(0),
        });
        let buffer = input.buffer();
        let changed = state.clone();
        buffer.connect_changed(move |_| {
            if !changed.applying.get() {
                changed.dismiss();
            }
        });
        let edited = state.clone();
        buffer.connect_end_user_action(move |_| {
            if edited.applying.get() || edited.undoing.get() {
                return;
            }
            let state = edited.clone();
            let generation = state.generation.get();
            glib::idle_add_local_once(move || {
                if state.generation.get() == generation {
                    state.update();
                }
            });
        });
        let selection = state.clone();
        buffer.connect_cursor_position_notify(move |buffer| {
            let moved = selection.context.borrow().as_ref().is_some_and(|context| {
                context.text == buffer_text(buffer)
                    && (context.caret != buffer.cursor_position() || buffer.has_selection())
            });
            if moved && !selection.applying.get() {
                selection.dismiss();
            }
        });
        let preedit = state.clone();
        input.connect_preedit_changed(move |_, text| {
            preedit.preedit.set(!text.is_empty());
            if !text.is_empty() {
                preedit.dismiss();
            }
        });
        let undo = state.clone();
        buffer.connect_undo(move |_| State::begin_undo(&undo));
        let redo = state.clone();
        buffer.connect_redo(move |_| State::begin_undo(&redo));
        let focus = gtk::EventControllerFocus::new();
        let blurred = state.clone();
        focus.connect_leave(move |_| blurred.dismiss());
        input.add_controller(focus);
        Self { root, state }
    }

    pub fn set_people(&self, people: Vec<Entry>) {
        *self.state.catalog.borrow_mut() = people;
    }

    pub fn handle_key(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
        if self.state.preedit.get()
            || !modifiers
                .intersection(
                    gtk::gdk::ModifierType::SHIFT_MASK
                        | gtk::gdk::ModifierType::CONTROL_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK,
                )
                .is_empty()
        {
            return false;
        }
        let count = self.state.results.borrow().len();
        if count == 0 {
            return false;
        }
        match key {
            gtk::gdk::Key::Up => self
                .state
                .selected
                .set((self.state.selected.get() + count - 1) % count),
            gtk::gdk::Key::Down => self
                .state
                .selected
                .set((self.state.selected.get() + 1) % count),
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::Tab => {
                let entry = self.state.results.borrow()[self.state.selected.get()].clone();
                self.state.insert(&entry);
                return true;
            }
            gtk::gdk::Key::Escape => {
                self.state.dismiss();
                return true;
            }
            gtk::gdk::Key::Left
            | gtk::gdk::Key::Right
            | gtk::gdk::Key::Home
            | gtk::gdk::Key::End => {
                self.state.dismiss();
                return false;
            }
            _ => return false,
        }
        self.state.render();
        true
    }
}

fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}

impl State {
    fn begin_undo(state: &Rc<Self>) {
        state.undoing.set(true);
        state.dismiss();
        let state = state.clone();
        glib::idle_add_local_once(move || state.undoing.set(false));
    }

    fn dismiss(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.context.borrow_mut().take();
        self.results.borrow_mut().clear();
        if let Some(root) = self.root.upgrade() {
            root.set_visible(false);
        }
    }

    fn update(self: &Rc<Self>) {
        let Some(input) = self.input.upgrade() else {
            return;
        };
        if self.preedit.get() || self.undoing.get() || !input.has_focus() {
            self.dismiss();
            return;
        }
        let buffer = input.buffer();
        let text = buffer_text(&buffer);
        let caret = buffer.cursor_position();
        let Some(token) = logic::token(&text, caret, buffer.has_selection()) else {
            self.dismiss();
            return;
        };
        let query = token.query.to_lowercase();
        *self.context.borrow_mut() = Some(Context { text, caret, token });
        *self.results.borrow_mut() = self
            .catalog
            .borrow()
            .iter()
            .filter(|entry| entry.display_name.to_lowercase().contains(&query))
            .take(6)
            .cloned()
            .collect();
        self.selected.set(0);
        self.render();
    }

    fn insert(&self, entry: &Entry) {
        let Some(input) = self.input.upgrade() else {
            return;
        };
        let Some(context) = self.context.borrow_mut().take() else {
            return;
        };
        let buffer = input.buffer();
        if self.preedit.get()
            || buffer.has_selection()
            || buffer.cursor_position() != context.caret
            || buffer_text(&buffer) != context.text
        {
            self.dismiss();
            return;
        }
        let suffix = context.text.chars().nth(context.token.end as usize);
        let replacement = format!(
            "{}{}",
            format!("@{}", entry.display_name),
            if suffix.is_none_or(|ch| !ch.is_whitespace()) {
                " "
            } else {
                ""
            }
        );
        let token = iris_chat_core::format_message_mention(
            entry.owner_pubkey_hex.clone(),
            entry.display_name.clone(),
        );
        if token.is_empty() {
            return;
        }
        self.applying.set(true);
        buffer.begin_user_action();
        let mut start = buffer.iter_at_offset(context.token.start);
        let mut end = buffer.iter_at_offset(context.token.end);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &replacement);
        buffer.place_cursor(&start);
        buffer.end_user_action();
        (self.inserted)(
            context.token.start,
            format!("@{}", entry.display_name),
            token,
        );
        self.applying.set(false);
        self.dismiss();
        input.grab_focus();
    }

    fn render(self: &Rc<Self>) {
        let Some(root) = self.root.upgrade() else {
            return;
        };
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        let results = self.results.borrow();
        for (index, entry) in results.iter().enumerate() {
            let duplicate = results
                .iter()
                .filter(|person| person.display_name == entry.display_name)
                .count()
                > 1;
            let label = if duplicate {
                format!("{} · {}", entry.display_name, &entry.owner_pubkey_hex[..8])
            } else {
                entry.display_name.clone()
            };
            let button = gtk::Button::with_label(&label);
            button.add_css_class("flat");
            if index == self.selected.get() {
                button.add_css_class("suggested-action");
            }
            button.set_halign(gtk::Align::Fill);
            button.set_focus_on_click(false);
            button.show_pointer_cursor();
            button.set_widget_name(&format!("chatMentionCompletion-{}", entry.owner_pubkey_hex));
            let state = self.clone();
            let entry = entry.clone();
            button.connect_clicked(move |_| state.insert(&entry));
            root.append(&button);
        }
        root.set_visible(!results.is_empty());
    }
}
