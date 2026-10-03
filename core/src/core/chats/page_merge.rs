use super::*;
use std::collections::HashMap;

// Both inputs are chronological, with insertion order breaking timestamp ties.
// Shared IDs anchor the stored page inside a possibly larger resident window.
// This preserves older cached rows, a partial cache, and an unsaved live tail;
// the stored value wins each overlap without moving it ahead of older rows.
pub(super) fn merge_stored_page(
    cached: Vec<ChatMessageSnapshot>,
    stored: Vec<ChatMessageSnapshot>,
) -> Vec<ChatMessageSnapshot> {
    let stored_ids: HashSet<_> = stored.iter().map(|message| message.id.clone()).collect();
    let mut page = Vec::with_capacity(cached.len() + stored.len());
    let positions: HashMap<_, _> = cached
        .iter()
        .enumerate()
        .filter(|(_, message)| stored_ids.contains(&message.id))
        .map(|(index, message)| (message.id.clone(), index))
        .collect();
    let mut cached = cached.into_iter().enumerate().peekable();
    let mut before_anchor = Vec::new();
    for message in stored {
        if let Some(anchor) = positions.get(&message.id) {
            while cached.peek().is_some_and(|(index, _)| index <= anchor) {
                if let Some((_, message)) = cached.next() {
                    if !stored_ids.contains(&message.id) {
                        page.push(message);
                    }
                }
            }
            page.append(&mut before_anchor);
            page.push(message);
        } else {
            before_anchor.push(message);
        }
    }
    page.append(&mut before_anchor);
    page.extend(
        cached
            .map(|(_, message)| message)
            .filter(|message| !stored_ids.contains(&message.id)),
    );
    page.sort_by_key(message_order);
    page
}
