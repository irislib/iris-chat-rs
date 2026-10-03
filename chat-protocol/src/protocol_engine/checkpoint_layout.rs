// Keep checkpoints ordinary JSON so existing clients and recovery tools can
// still read them. Remember only whitespace boundaries, never another copy of
// secret state. Reuse these boundaries so a one-byte record overflow consumes
// nearby padding instead of shifting every subsequent record by a whole page.
#[derive(Default)]
struct ProtocolCheckpointLayout {
    record_ends: Vec<usize>,
    bytes: usize,
}

const CHECKPOINT_PAGE: usize = 4096;
const CHECKPOINT_RESERVE: usize = 64 * 1024;

struct CheckpointRecord {
    end: usize,
    padded_end: usize,
    large: bool,
}

fn checkpoint_records(json: &str) -> Vec<CheckpointRecord> {
    let mut records = Vec::new();
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in json.bytes().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == b'\\' {
                escaped = true;
            } else if character == b'"' {
                quoted = false;
            }
            continue;
        }
        match character {
            b'"' => quoted = true,
            b'{' | b'[' => stack.push((character, index)),
            b'}' | b']' => {
                if let Some((kind, start)) = stack.pop() {
                    if kind == b'{'
                        && stack.len() <= 3
                        && stack.last().is_some_and(|(kind, _)| *kind == b'[')
                    {
                        let end = index + 1;
                        let padding = json.as_bytes()[end..]
                            .iter()
                            .take_while(|byte| byte.is_ascii_whitespace())
                            .count();
                        records.push(CheckpointRecord {
                            end,
                            padded_end: end + padding,
                            large: index - start >= CHECKPOINT_PAGE / 2,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    records
}

impl ProtocolCheckpointLayout {
    fn from_json(json: &str) -> Self {
        if json.len() < CHECKPOINT_RESERVE {
            return Self::default();
        }
        Self {
            record_ends: checkpoint_records(json)
                .iter()
                .map(|record| record.padded_end)
                .collect(),
            bytes: json.len(),
        }
    }
}

fn layout_protocol_checkpoint(
    json: String,
    previous: &ProtocolCheckpointLayout,
) -> (String, ProtocolCheckpointLayout) {
    if json.len() < CHECKPOINT_RESERVE {
        return (json, ProtocolCheckpointLayout::default());
    }
    let records = checkpoint_records(&json);
    // Parsed legacy/pretty/padded JSON remains readable without growing again.
    // Production serialization is compact and has no whitespace after objects.
    if records.iter().any(|record| record.end != record.padded_end)
        || json.ends_with(char::is_whitespace)
    {
        let layout = ProtocolCheckpointLayout::from_json(&json);
        return (json, layout);
    }
    // Reclaim excessive slack after a major state reduction. Ordinary receive
    // and acknowledgement mutations retain their existing allocation.
    let reuse = previous.bytes <= json.len().saturating_mul(3) + CHECKPOINT_RESERVE;
    let mut output = String::with_capacity(json.len() + CHECKPOINT_RESERVE);
    let mut layout = ProtocolCheckpointLayout::default();
    let mut copied = 0;
    for (index, record) in records.iter().enumerate() {
        output.push_str(&json[copied..record.end]);
        let target = if reuse {
            previous.record_ends.get(index).copied()
        } else {
            None
        }
        .unwrap_or_else(|| {
            if record.large {
                output.len().div_ceil(CHECKPOINT_PAGE) * CHECKPOINT_PAGE
            } else {
                output.len()
            }
        });
        output.extend(std::iter::repeat_n(
            ' ',
            target.saturating_sub(output.len()),
        ));
        layout.record_ends.push(output.len());
        copied = record.end;
    }
    output.push_str(&json[copied..]);
    let target = if reuse && output.len() <= previous.bytes {
        previous.bytes
    } else {
        output.len().div_ceil(CHECKPOINT_RESERVE) * CHECKPOINT_RESERVE
    };
    output.extend(std::iter::repeat_n(' ', target - output.len()));
    layout.bytes = output.len();
    (output, layout)
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
        let aligned =
            layout_protocol_checkpoint(raw.clone(), &ProtocolCheckpointLayout::default()).0;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&aligned).unwrap(),
            state
        );
        assert!(
            aligned.len() < raw.len() * 3 + 65536,
            "padding must stay bounded"
        );
        assert_eq!(
            layout_protocol_checkpoint(aligned.clone(), &ProtocolCheckpointLayout::default()).0,
            aligned,
            "layout must not grow on repeated saves"
        );
        assert_eq!(
            layout_protocol_checkpoint(
                "{\"small\":true}".into(),
                &ProtocolCheckpointLayout::default()
            )
            .0,
            "{\"small\":true}"
        );
    }
    #[test]
    fn record_growth_across_a_page_boundary_does_not_move_unchanged_sessions() {
        let mut state = serde_json::json!({"sessions": (0..64).map(|i|
            serde_json::json!({"id": i, "ratchet": "a".repeat(2300)})
        ).collect::<Vec<_>>()});
        state["sessions"][0]["ratchet"] = serde_json::Value::String(String::new());
        let empty = state.to_string();
        let first_end = empty.find('}').unwrap() + 1;
        state["sessions"][0]["ratchet"] = serde_json::Value::String("a".repeat(4096 - first_end));
        let (before, layout) =
            layout_protocol_checkpoint(state.to_string(), &ProtocolCheckpointLayout::default());
        let grown = format!("{}b", state["sessions"][0]["ratchet"].as_str().unwrap());
        state["sessions"][0]["ratchet"] = serde_json::Value::String(grown);
        let reloaded_layout = ProtocolCheckpointLayout::from_json(&before);
        let (after, _) = layout_protocol_checkpoint(state.to_string(), &reloaded_layout);
        assert_eq!(
            after,
            layout_protocol_checkpoint(state.to_string(), &layout).0
        );
        assert_eq!(
            before.len(),
            after.len(),
            "retain the SQLite TEXT allocation"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&after).unwrap(),
            state
        );
        let changed = before
            .as_bytes()
            .chunks(4096)
            .zip(after.as_bytes().chunks(4096))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed <= 3,
            "a one-byte ratchet growth moved {changed} pages"
        );
        let grown = state["sessions"][0]["ratchet"].as_str().unwrap();
        state["sessions"][0]["ratchet"] =
            serde_json::Value::String(grown[..grown.len() - 1].into());
        let (acknowledged, _) = layout_protocol_checkpoint(
            state.to_string(),
            &ProtocolCheckpointLayout::from_json(&after),
        );
        assert_eq!(before.len(), acknowledged.len());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&acknowledged).unwrap(),
            state
        );
        let changed = after
            .as_bytes()
            .chunks(4096)
            .zip(acknowledged.as_bytes().chunks(4096))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed <= 3,
            "shrinking a record after reopen moved {changed} pages"
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
        let (aligned_before, layout) =
            layout_protocol_checkpoint(before, &ProtocolCheckpointLayout::default());
        let (aligned_after, _) = layout_protocol_checkpoint(after, &layout);
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
