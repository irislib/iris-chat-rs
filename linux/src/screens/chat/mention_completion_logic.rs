#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Token {
    pub start: i32,
    pub end: i32,
    pub query: String,
}
pub(super) fn token(text: &str, caret: i32, selected: bool) -> Option<Token> {
    if selected || caret < 1 {
        return None;
    }
    let chars: Vec<_> = text.chars().collect();
    let caret = caret as usize;
    if caret > chars.len() {
        return None;
    }
    let start = chars[..caret].iter().rposition(|c| *c == '@')?;
    if start > 0 && !chars[start - 1].is_whitespace() {
        return None;
    }
    let query = &chars[start + 1..caret];
    if query.len() > 64
        || query
            .iter()
            .any(|c| !c.is_alphanumeric() && ![' ', '_', '-'].contains(c))
    {
        return None;
    }
    let mut end = caret;
    while end < chars.len() && (chars[end].is_alphanumeric() || ['_', '-'].contains(&chars[end])) {
        end += 1;
        if end - start > 65 {
            return None;
        }
    }
    Some(Token {
        start: start as i32,
        end: end as i32,
        query: query.iter().collect(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bare_at_and_names_work_without_matching_emails_or_selected_text() {
        assert_eq!(token("@", 1, false).unwrap().query, "");
        assert_eq!(token("👋 @Carol S", 10, false).unwrap().query, "Carol S");
        assert!(token("x@y", 3, false).is_none());
        assert!(token("@", 1, true).is_none());
        assert!(token(&format!("@{}", "a".repeat(70)), 71, false).is_none());
    }
}
