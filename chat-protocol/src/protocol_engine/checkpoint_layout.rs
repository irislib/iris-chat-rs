// Keep checkpoints ordinary JSON so existing clients and recovery tools can
// still read them. Align large array records so a length change in one session
// does not move every later record; SQLite can update only the changed pages.
fn layout_protocol_checkpoint(json: String) -> String {
    const PAGE: usize = 4096;
    const RESERVE: usize = 64 * 1024;
    if json.len() < RESERVE {
        return json;
    }
    let mut output = String::with_capacity(json.len() + RESERVE);
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in json.char_indices() {
        output.push(character);
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '{' | '[' => stack.push((character, index)),
            '}' | ']' => {
                if let Some((kind, start)) = stack.pop() {
                    if kind == '{'
                        && index - start >= PAGE / 2
                        && stack.len() <= 3
                        && stack.last().is_some_and(|(kind, _)| *kind == '[')
                    {
                        output.extend(std::iter::repeat_n(
                            ' ',
                            output.len().div_ceil(PAGE) * PAGE - output.len(),
                        ));
                    }
                }
            }
            // Checkpoints come from compact serde_json output. Existing padded
            // or pretty JSON is already readable and must not grow on re-layout.
            ' ' | '\n' | '\r' | '\t' => return json,
            _ => {}
        }
    }
    output.extend(std::iter::repeat_n(
        ' ',
        output.len().div_ceil(RESERVE) * RESERVE - output.len(),
    ));
    output
}

#[cfg(test)]
mod checkpoint_layout_tests {
    use super::*;

    #[test]
    fn layout_keeps_legacy_json_unicode_and_escape_sequences_readable() {
        let state = serde_json::json!({"version": 1, "sessions": (0..64).map(|i|
            serde_json::json!({"id": i, "message": r#"escaped " bracket } [ \"#.repeat(160), "name": "🦀 ä"})
        ).collect::<Vec<_>>()});
        let raw = state.to_string();
        let aligned = layout_protocol_checkpoint(raw.clone());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&aligned).unwrap(),
            state
        );
        assert!(
            aligned.len() < raw.len() * 3 + 65536,
            "padding must stay bounded"
        );
        assert_eq!(
            layout_protocol_checkpoint(aligned.clone()),
            aligned,
            "layout must not grow on repeated saves"
        );
        assert_eq!(
            layout_protocol_checkpoint("{\"small\":true}".into()),
            "{\"small\":true}"
        );
    }
    #[test]
    fn record_alignment_limits_writes_when_a_session_changes_length() {
        let mut state = serde_json::json!({"sessions": (0..64).map(|i|
            serde_json::json!({"id": i, "ratchet": "a".repeat(2300)})
        ).collect::<Vec<_>>()});
        let before = state.to_string();
        state["sessions"][0]["ratchet"] = serde_json::Value::String("b".repeat(2301));
        let after = state.to_string();
        let changed_pages = |a: &str, b: &str| {
            a.as_bytes()
                .chunks(4096)
                .zip(b.as_bytes().chunks(4096))
                .filter(|(a, b)| a != b)
                .count()
        };
        let compact_changes = changed_pages(&before, &after);
        let aligned_before = layout_protocol_checkpoint(before);
        let aligned_after = layout_protocol_checkpoint(after);
        assert_eq!(aligned_before.len(), aligned_after.len());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&aligned_after).unwrap(),
            state
        );
        assert!(
            compact_changes > 30,
            "control must exercise shifted subsequent sessions"
        );
        assert_eq!(changed_pages(&aligned_before, &aligned_after), 1);
    }
}
