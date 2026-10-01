use crate::{AccountSnapshot, ChatKind, ChatThreadSnapshot};

pub(crate) const NOTE_TO_SELF: &str = "Note to self";

/// Expose self-chat before its first message without persisting an empty thread.
pub(crate) fn include_note_to_self(chats: &mut Vec<ChatThreadSnapshot>, account: &AccountSnapshot) {
    if chats
        .iter()
        .any(|chat| chat.chat_id == account.public_key_hex)
    {
        return;
    }
    chats.push(ChatThreadSnapshot {
        social_connection: Some(crate::SocialConnectionSnapshot {
            badge: Some(crate::SocialBadge::Following),
            follow_distance: Some(0),
            followed_by_friends: 0,
            description: "You".into(),
        }),
        chat_id: account.public_key_hex.clone(),
        kind: ChatKind::Direct,
        display_name: NOTE_TO_SELF.to_string(),
        nickname: None,
        contact_note: None,
        profile_name: Some(account.display_name.clone()),
        subtitle: None,
        picture_url: account.picture_url.clone(),
        about: account.about.clone(),
        member_count: 0,
        last_message_preview: None,
        last_message_at_secs: None,
        last_message_is_outgoing: None,
        last_message_delivery: None,
        unread_count: 0,
        is_typing: false,
        is_muted: false,
        is_pinned: false,
        draft: String::new(),
        is_request: false,
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NameMatchRank {
    Exact,
    WordPrefix,
    Substring,
    OtherField,
}

/// Shared contact and people matching, with names ahead of descriptive fields.
pub(crate) struct SearchQuery {
    text: String,
    compact: String,
    terms: Vec<(String, String)>,
}

impl SearchQuery {
    pub(crate) fn new(query: &str) -> Self {
        let text = query.trim().to_lowercase();
        Self {
            compact: compact_search_text(&text),
            terms: text
                .split_whitespace()
                .map(|term| (term.to_string(), compact_search_text(term)))
                .collect(),
            text,
        }
    }

    pub(crate) fn matches(&self, fields: &[&str]) -> bool {
        let fields = fields
            .iter()
            .flat_map(|field| [field.to_lowercase(), compact_search_text(field)])
            .collect::<Vec<_>>();
        !self.terms.is_empty()
            && self.terms.iter().all(|(term, compact)| {
                fields.iter().any(|field| {
                    field.contains(term) || (!compact.is_empty() && field.contains(compact))
                })
            })
    }

    pub(crate) fn name_rank(&self, names: &[&str]) -> NameMatchRank {
        let mut rank = if self.matches(names) {
            NameMatchRank::Substring
        } else {
            NameMatchRank::OtherField
        };
        for name in names {
            let name = name.to_lowercase();
            let compact = compact_search_text(&name);
            if name == self.text || (!self.compact.is_empty() && compact == self.compact) {
                return NameMatchRank::Exact;
            }
            // Match surnames and punctuation-separated words as well as the
            // first word. Keep joined-name searches such as "AnnaSm" working.
            let words = name
                .split(|character: char| !character.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>();
            if !self.terms.is_empty()
                && (name.starts_with(&self.text)
                    || (!self.compact.is_empty() && compact.starts_with(&self.compact))
                    || self
                        .terms
                        .iter()
                        .all(|(term, _)| words.iter().any(|word| word.starts_with(term))))
            {
                rank = NameMatchRank::WordPrefix;
            }
        }
        rank
    }
}

fn compact_search_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn filter_threads_for_search(
    chat_list: &[ChatThreadSnapshot],
    query: &str,
) -> (Vec<ChatThreadSnapshot>, Vec<ChatThreadSnapshot>) {
    let query = SearchQuery::new(query);
    let mut matches = Vec::new();
    for chat in chat_list {
        let names = [
            chat.display_name.as_str(),
            chat.nickname.as_deref().unwrap_or_default(),
            chat.profile_name.as_deref().unwrap_or_default(),
        ];
        let fields = [
            names.as_slice(),
            &[
                chat.about.as_deref().unwrap_or_default(),
                chat.subtitle.as_deref().unwrap_or_default(),
                &chat.draft,
                &chat.chat_id,
            ],
        ]
        .concat();
        if query.matches(&fields) {
            matches.push((query.name_rank(&names), chat));
        }
    }
    // Stable sorting preserves the existing chat order for equally good hits.
    matches.sort_by_key(|(rank, _)| *rank);
    let mut contacts = Vec::new();
    let mut groups = Vec::new();
    for (_, chat) in matches {
        match chat.kind {
            ChatKind::Direct => contacts.push(chat.clone()),
            ChatKind::Group => groups.push(chat.clone()),
        }
    }
    (contacts, groups)
}
