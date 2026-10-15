use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct Span {
    pub range: Range<usize>,
    pub label: String,
    pub token: String,
}

#[derive(Default)]
pub(super) struct MentionDraft {
    text: String,
    spans: Vec<Span>,
}

impl MentionDraft {
    pub fn restore(&mut self, wire: &str, spans: Vec<Span>) -> String {
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
    pub fn changed(&mut self, text: &str) {
        if self.text == text {
            return;
        }
        let prefix = self
            .text
            .bytes()
            .zip(text.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = self
            .text
            .bytes()
            .rev()
            .zip(text.bytes().rev())
            .take(self.text.len().min(text.len()) - prefix)
            .take_while(|(a, b)| a == b)
            .count();
        let end = self.text.len() - suffix;
        let delta = text.len() as isize - self.text.len() as isize;
        self.spans.retain_mut(|span| {
            if span.range.end == prefix
                && text
                    .get(prefix..)
                    .and_then(|s| s.chars().next())
                    .is_some_and(char::is_alphanumeric)
            {
                return false;
            }
            if span.range.end <= prefix {
                return true;
            }
            if span.range.start >= end {
                span.range = span.range.start.checked_add_signed(delta).unwrap()
                    ..span.range.end.checked_add_signed(delta).unwrap();
                return true;
            }
            false
        });
        self.text = text.to_string();
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
        draft.changed(&format!("Hello {visible}"));
        assert_eq!(draft.wire_text(), "Hello 👋 token!");
    }
    #[test]
    fn editing_a_label_does_not_reconstruct_identity_from_a_name() {
        let mut draft = MentionDraft::default();
        draft.changed("@Alex");
        draft.insert(0, "@Alex".into(), "alice-id".into());
        assert_eq!(draft.wire_text(), "alice-id");
        draft.changed("@Alexandra");
        assert_eq!(draft.wire_text(), "@Alexandra");
        draft.changed("@Alex");
        assert_eq!(draft.wire_text(), "@Alex");
    }
}
