use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Span {
    pub range: Range<usize>,
    pub label: String,
    pub token: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Snapshot {
    text: String,
    spans: Vec<Span>,
}

#[derive(Default)]
pub(super) struct MentionDraft {
    text: String,
    spans: Vec<Span>,
    expected: Option<String>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    action_depth: usize,
    checkpointed: bool,
    replay: Option<(bool, Snapshot)>,
    pub restoring: bool,
}

impl MentionDraft {
    pub fn restore(&mut self, wire: &str, spans: Vec<Span>) -> String {
        self.undo.clear();
        self.redo.clear();
        self.expected = None;
        let mut text = wire.to_string();
        self.spans.clear();
        let mut shift = 0isize;
        for span in spans {
            let start = span.range.start.checked_add_signed(shift).unwrap();
            let end = span.range.end.checked_add_signed(shift).unwrap();
            text.replace_range(start..end, &span.label);
            shift += span.label.len() as isize - (end - start) as isize;
            self.spans.push(Span {
                range: start..start + span.label.len(),
                ..span
            });
        }
        self.text = text.clone();
        text
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            spans: self.spans.clone(),
        }
    }
    pub fn begin_action(&mut self) {
        if self.action_depth == 0 {
            self.checkpointed = false;
        }
        self.action_depth += 1;
    }
    pub fn end_action(&mut self) {
        self.action_depth = self.action_depth.saturating_sub(1);
    }
    pub fn prepare_edit(&mut self, range: Range<usize>, replacement: &str) {
        if self.restoring || self.replay.is_some() {
            return;
        }
        if self.action_depth == 0 || !self.checkpointed {
            self.undo.push(self.snapshot());
            self.redo.clear();
            self.checkpointed = true;
        }
        let mut expected = self.text.clone();
        if expected.get(range.clone()).is_none() {
            self.spans.clear();
            self.expected = None;
            return;
        }
        expected.replace_range(range.clone(), replacement);
        self.expected = Some(expected);
        let delta = replacement.len() as isize - (range.end - range.start) as isize;
        let word = replacement
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        self.spans.retain_mut(|span| {
            if span.range.end == range.start && word {
                return false;
            }
            if span.range.end <= range.start {
                return true;
            }
            if span.range.start >= range.end {
                span.range = span.range.start.checked_add_signed(delta).unwrap()
                    ..span.range.end.checked_add_signed(delta).unwrap();
                return true;
            }
            false
        });
    }
    pub fn changed(&mut self, text: &str) {
        if self.restoring {
            return;
        }
        if self.text != text && self.replay.is_none() && self.expected.as_deref() != Some(text) {
            // A mutation without an editor range cannot safely carry identities.
            self.spans.clear();
            self.undo.clear();
            self.redo.clear();
        }
        self.expected = None;
        self.text = text.to_owned();
    }
    pub fn is_replaying(&self) -> bool {
        self.replay.is_some()
    }
    pub fn begin_replay(&mut self, redo: bool) {
        self.replay = Some((redo, self.snapshot()));
    }
    pub fn finish_replay(&mut self, text: &str) {
        let Some((redo, before)) = self.replay.take() else {
            return;
        };
        // GTK can coalesce adjacent typing actions into one undo unit. Only an
        // explicit native undo/redo may traverse these recorded editor states;
        // normal typing never restores identity by matching a label.
        let stack = if redo { &mut self.redo } else { &mut self.undo };
        if let Some(index) = stack.iter().rposition(|snapshot| snapshot.text == text) {
            let restored = stack[index].clone();
            stack.truncate(index);
            self.text = restored.text;
            self.spans = restored.spans;
            if redo {
                self.undo.push(before);
            } else {
                self.redo.push(before);
            }
        } else {
            self.text = text.into();
            self.spans.clear();
            self.undo.clear();
            self.redo.clear();
        }
        self.expected = None;
    }
    pub fn insert(&mut self, start: usize, label: String, token: String) {
        let range = start..start + label.len();
        self.spans
            .retain(|span| span.range.end <= range.start || span.range.start >= range.end);
        self.spans.push(Span {
            range,
            label,
            token,
        });
        self.spans.sort_by_key(|span| span.range.start);
    }
    pub fn wire_text(&self) -> String {
        let mut text = self.text.clone();
        for span in self.spans.iter().rev() {
            if text.get(span.range.clone()) == Some(&span.label) {
                text.replace_range(span.range.clone(), &span.token);
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_restore_with_stable_identity_and_unicode_offsets() {
        let mut draft = MentionDraft::default();
        let visible = draft.restore(
            "👋 token!",
            vec![Span {
                range: 5..10,
                label: "@New name".into(),
                token: "token".into(),
            }],
        );
        assert_eq!(visible, "👋 @New name!");
        draft.prepare_edit(0..0, "Hello ");
        draft.changed(&format!("Hello {visible}"));
        assert_eq!(draft.wire_text(), "Hello 👋 token!");
    }
    #[test]
    fn editing_a_label_does_not_reconstruct_identity_from_a_name() {
        let mut draft = MentionDraft::default();
        draft.changed("@Alex");
        draft.insert(0, "@Alex".into(), "alice-id".into());
        assert_eq!(draft.wire_text(), "alice-id");
        draft.prepare_edit(5..5, "andra");
        draft.changed("@Alexandra");
        assert_eq!(draft.wire_text(), "@Alexandra");
        draft.changed("@Alex");
        assert_eq!(draft.wire_text(), "@Alex");
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    fn two_people() -> MentionDraft {
        let mut draft = MentionDraft::default();
        draft.changed("@Alex @Alex ");
        draft.insert(0, "@Alex".into(), "alice".into());
        draft.insert(6, "@Alex".into(), "bob".into());
        draft
    }
    #[test]
    fn actual_range_distinguishes_equal_labels_and_native_undo_restores_them() {
        for (range, expected) in [(0..6, "bob "), (6..12, "alice ")] {
            let mut draft = two_people();
            draft.begin_action();
            draft.prepare_edit(range, "");
            draft.changed("@Alex ");
            draft.end_action();
            assert_eq!(draft.wire_text(), expected);
            draft.begin_replay(false);
            draft.changed("@Alex @Alex ");
            draft.finish_replay("@Alex @Alex ");
            assert_eq!(draft.wire_text(), "alice bob ");
            draft.begin_replay(true);
            draft.changed("@Alex ");
            draft.finish_replay("@Alex ");
            assert_eq!(draft.wire_text(), expected);
        }
    }
    #[test]
    fn replacing_the_same_visible_text_requires_a_new_identity_choice() {
        let mut draft = two_people();
        draft.begin_action();
        draft.prepare_edit(0..5, "");
        draft.changed(" @Alex ");
        draft.prepare_edit(0..0, "@Alex");
        draft.changed("@Alex @Alex ");
        draft.end_action();
        assert_eq!(draft.wire_text(), "@Alex bob ");
        draft.begin_replay(false);
        draft.changed("@Alex @Alex ");
        draft.finish_replay("@Alex @Alex ");
        assert_eq!(draft.wire_text(), "alice bob ");
        draft.begin_replay(true);
        draft.changed("@Alex @Alex ");
        draft.finish_replay("@Alex @Alex ");
        assert_eq!(draft.wire_text(), "@Alex bob ");
    }
    #[test]
    fn undo_restores_edited_identity_and_coalesced_prefix_typing() {
        let mut draft = two_people();
        draft.begin_action();
        draft.prepare_edit(1..2, "Z");
        draft.changed("@Zlex @Alex ");
        draft.end_action();
        assert_eq!(draft.wire_text(), "@Zlex bob ");
        draft.begin_replay(false);
        draft.changed("@Alex @Alex ");
        draft.finish_replay("@Alex @Alex ");
        assert_eq!(draft.wire_text(), "alice bob ");
        for prefix in ["H", "Hi"] {
            let old = draft.text.clone();
            draft.begin_action();
            draft.prepare_edit(0..0, prefix);
            draft.changed(&format!("{prefix}{old}"));
            draft.end_action();
        }
        draft.begin_replay(false);
        draft.changed("@Alex @Alex ");
        draft.finish_replay("@Alex @Alex ");
        assert_eq!(draft.wire_text(), "alice bob ");
        draft.begin_replay(true);
        draft.changed("HiH@Alex @Alex ");
        draft.finish_replay("HiH@Alex @Alex ");
        assert_eq!(draft.wire_text(), "HiHalice bob ");
    }
}
