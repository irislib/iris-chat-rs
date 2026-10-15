use super::mention_draft::MentionDraft;
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};

/// Observe native ranges before GTK mutates the buffer; duplicate visible
/// labels must never be reconciled by a text diff.
pub(super) fn install(
    buffer: &gtk::TextBuffer,
    draft: Rc<RefCell<MentionDraft>>,
    replayed: impl Fn(String) + 'static,
) {
    let begin = draft.clone();
    buffer.connect_begin_user_action(move |_| begin.borrow_mut().begin_action());
    let end = draft.clone();
    buffer.connect_end_user_action(move |_| end.borrow_mut().end_action());
    let inserted = draft.clone();
    buffer.connect_insert_text(move |buffer, at, text| {
        let start = buffer.text(&buffer.start_iter(), at, true).len();
        inserted.borrow_mut().prepare_edit(start..start, text);
    });
    let deleted = draft.clone();
    buffer.connect_delete_range(move |buffer, start, end| {
        let from = buffer.text(&buffer.start_iter(), start, true).len();
        let to = from + buffer.text(start, end, true).len();
        deleted.borrow_mut().prepare_edit(from..to, "");
    });
    let replayed = Rc::new(replayed);
    for (signal, redo) in [("undo", false), ("redo", true)] {
        let before = draft.clone();
        buffer.connect_local(signal, false, move |values| {
            let buffer = values[0]
                .get::<gtk::TextBuffer>()
                .expect("text buffer signal");
            if if redo {
                buffer.can_redo()
            } else {
                buffer.can_undo()
            } {
                before.borrow_mut().begin_replay(redo);
            }
            None
        });
        let after = draft.clone();
        let replayed = replayed.clone();
        buffer.connect_local(signal, true, move |values| {
            let buffer = values[0]
                .get::<gtk::TextBuffer>()
                .expect("text buffer signal");
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            after.borrow_mut().finish_replay(&text);
            let wire = after.borrow().wire_text();
            replayed(wire);
            None
        });
    }
}
