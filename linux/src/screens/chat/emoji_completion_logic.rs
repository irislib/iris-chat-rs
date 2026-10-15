#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Token {
    pub start: i32,
    pub end: i32,
    pub query: String,
    pub closed: bool,
}

pub(super) fn token(text: &str, caret: i32, selected: bool) -> Option<Token> {
    if selected || caret < 0 {
        return None;
    }
    let caret_byte = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .nth(caret as usize)?;
    let prefix = &text[..caret_byte];
    let start_byte = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let raw = prefix.get(start_byte..)?.strip_prefix(':')?;
    let closed = raw.ends_with(':');
    let query = if closed { &raw[..raw.len() - 1] } else { raw };
    let query_len = query.chars().take(65).count();
    if query_len > 64 || (closed && query.is_empty()) || !query.chars().all(token_char) {
        return None;
    }
    let mut end = caret;
    if !closed {
        let mut count = query_len;
        for ch in text[caret_byte..].chars() {
            if token_char(ch) {
                count += 1;
                if count > 64 {
                    return None;
                }
                end += 1;
            } else {
                if ch == ':' {
                    end += 1;
                }
                break;
            }
        }
    }
    Some(Token {
        start: text[..start_byte].chars().count() as i32,
        end,
        query: query.to_string(),
        closed,
    })
}

fn token_char(ch: char) -> bool {
    ch.is_alphanumeric() || "_+-".contains(ch)
}
fn normalize(text: &str) -> String {
    text.to_lowercase().replace('-', "_")
}

#[derive(Clone)]
pub(super) struct Entry {
    pub emoji: String,
    pub names: Vec<String>,
}

impl Entry {
    pub fn exact(&self, query: &str) -> bool {
        self.names
            .iter()
            .any(|name| normalize(name) == normalize(query))
    }
}

pub(super) fn search(query: &str, entries: &[Entry]) -> Vec<Entry> {
    if query.chars().count() < 2 {
        return vec![];
    }
    let query = normalize(query);
    let mut matches: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let names: Vec<_> = entry.names.iter().map(|name| normalize(name)).collect();
            let rank = if entry.exact(&query) {
                0
            } else if names.iter().any(|name| name.starts_with(&query)) {
                1
            } else if names.iter().any(|name| name.contains(&query)) {
                2
            } else {
                return None;
            };
            Some((rank, index, entry))
        })
        .collect();
    matches.sort_by_key(|(rank, index, _)| (*rank, *index));
    matches
        .into_iter()
        .take(6)
        .map(|(_, _, entry)| entry.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_times_repeated_delimiters_and_selection_are_literal() {
        for text in [
            "https://iris.to/:sm",
            "12:30",
            "word:sm",
            ":smile::",
            "::smile:",
        ] {
            assert!(token(text, text.chars().count() as i32, false).is_none());
        }
        assert!(token(":sm", 3, true).is_none());
        assert!(token(":sm", 99, false).is_none());
        assert!(token(&":".repeat(100_000), 100_000, false).is_none());
        assert!(token(&format!(":sm{}", "x".repeat(100_000)), 3, false).is_none());
    }

    #[test]
    fn gtk_character_offsets_preserve_unicode_and_closed_suffix() {
        let text = "👩🏽‍💻 :smile: after";
        let result = token(text, "👩🏽‍💻 :sm".chars().count() as i32, false).unwrap();
        assert_eq!(result.query, "sm");
        let before: String = text.chars().take(result.start as usize).collect();
        let after: String = text.chars().skip(result.end as usize).collect();
        assert_eq!(format!("{before}😄{after}"), "👩🏽‍💻 😄 after");
        assert!(token(":fire:", 6, false).unwrap().closed);
    }

    #[test]
    fn exact_codes_rank_first_and_short_queries_do_not_open() {
        let entries = vec![
            Entry {
                emoji: "😃".into(),
                names: vec!["smiley".into()],
            },
            Entry {
                emoji: "😄".into(),
                names: vec!["smile".into()],
            },
        ];
        assert!(search("s", &entries).is_empty());
        assert_eq!(search("smile", &entries)[0].emoji, "😄");
        assert!(search("no_match", &entries).is_empty());
    }
}
