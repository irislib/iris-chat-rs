//! Identity-bearing mentions remain ordinary encrypted message content. The
//! Markdown link is readable by older clients and survives history sync, drafts,
//! edits and attachments without a new event kind or public recipient tags.

use nostr::{PublicKey, ToBech32};

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct MessageMention {
    /// UTF-16 offsets into the encoded body, shared by all native text editors.
    pub start: u32,
    pub length: u32,
    pub owner_pubkey_hex: String,
    /// Includes the visible @ prefix. This label is not identity verification.
    pub label: String,
}

#[uniffi::export]
pub fn format_message_mention(owner_pubkey_hex: String, name: String) -> String {
    let Ok(owner) = PublicKey::from_hex(&owner_pubkey_hex) else {
        return String::new();
    };
    let npub = owner
        .to_bech32()
        .expect("public key encoding is infallible");
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '[' | ']' | '\\'))
        .take(80)
        .collect();
    let name = name.trim().trim_start_matches('@');
    let label = if name.is_empty() { &npub[..12] } else { name };
    format!("[@{label}](nostr:{npub})")
}

#[uniffi::export]
pub fn message_mentions(text: String) -> Vec<MessageMention> {
    mentions(&text)
        .into_iter()
        .map(|(start, end, owner, label)| MessageMention {
            start: text[..start].encode_utf16().count() as u32,
            length: text[start..end].encode_utf16().count() as u32,
            owner_pubkey_hex: owner.to_hex(),
            label: label.to_owned(),
        })
        .collect()
}

/// Readable previews and quoted/forwarded content must not expose wire syntax.
#[uniffi::export]
pub fn display_message_mentions(text: String) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, _, label) in mentions(&text) {
        output.push_str(&text[cursor..start]);
        output.push_str(label);
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

pub(crate) fn mentions_owner(text: &str, owner: &str) -> bool {
    // Reply quotations and forwards describe someone else's words. They do not
    // turn an earlier mention into a fresh notification.
    let body = crate::core::strip_reply_quote(text);
    if body.starts_with("Forwarded:\n") {
        return false;
    }
    mentions(body)
        .iter()
        .any(|(_, _, key, _)| key.to_hex() == owner)
}

fn mentions(text: &str) -> Vec<(usize, usize, PublicKey, &str)> {
    let mut output = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("[@") {
        let start = cursor + offset;
        cursor = start + 2;
        let label_window_end = (cursor + 328).min(text.len());
        let label_window = &text.as_bytes()[cursor..label_window_end];
        let Some(label_end) = label_window
            .windows(8)
            .position(|w| w == b"](nostr:")
            .map(|i| cursor + i)
        else {
            continue;
        };
        let label = &text[start + 1..label_end];
        if label.len() > 321
            || label.len() < 2
            || label
                .chars()
                .any(|c| c.is_control() || matches!(c, '[' | ']' | '\\'))
        {
            continue;
        }
        let key_start = label_end + "](nostr:".len();
        let end = key_start + 63;
        if text.as_bytes().get(end) != Some(&b')') {
            continue;
        }
        let Some(key) = text.get(key_start..end) else {
            continue;
        };
        // One canonical, checksummed identifier; no relay hints or arbitrary URL.
        if key.len() != 63 || !key.starts_with("npub1") {
            continue;
        }
        let Ok(owner) = PublicKey::parse(key) else {
            continue;
        };
        output.push((start, end + 1, owner, label));
        cursor = end + 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    const ALICE: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const BOB: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

    #[test]
    fn identities_survive_equal_names_and_unicode_offsets() {
        let a = format_message_mention(ALICE.into(), "Alex 💜".into());
        let b = format_message_mention(BOB.into(), "Alex 💜".into());
        let text = format!("👋 {a}, {b}!");
        let found = message_mentions(text.clone());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].start, 3);
        assert_eq!(found[0].length as usize, a.encode_utf16().count());
        assert_eq!(found[0].owner_pubkey_hex, ALICE);
        assert_eq!(found[1].owner_pubkey_hex, BOB);
        assert_eq!(
            display_message_mentions(text.clone()),
            "👋 @Alex 💜, @Alex 💜!"
        );
        assert!(mentions_owner(&text, ALICE));
    }

    #[test]
    fn ordinary_names_links_and_invalid_identifiers_do_not_notify() {
        for text in [
            "Hello @Alex",
            "[Alex](nostr:npub1bad)",
            "[@Alex](https://example.com)",
            "[@Alex](nostr:npub1bad)",
        ] {
            assert!(!mentions_owner(text, ALICE));
            assert_eq!(display_message_mentions(text.into()), text);
        }
        assert!(format_message_mention("invalid".into(), "Alex".into()).is_empty());
    }

    #[test]
    fn quoted_and_forwarded_mentions_do_not_notify_again() {
        let mention = format_message_mention(ALICE.into(), "Alice".into());
        assert!(!mentions_owner(
            &format!("↩ Bob: {mention}\n\nThanks"),
            ALICE
        ));
        assert!(!mentions_owner(&format!("Forwarded:\n\n{mention}"), ALICE));
        assert!(mentions_owner(
            &format!("↩ Bob: Earlier\n\nHello {mention}"),
            ALICE
        ));
    }

    #[test]
    fn malformed_prefixes_are_bounded_and_do_not_hide_a_valid_identity() {
        let valid = format_message_mention(ALICE.into(), "Alice".into());
        let text = format!("{}{}", "[@".repeat(16_000), valid);
        assert_eq!(message_mentions(text).len(), 1);
    }

    #[test]
    fn labels_cannot_break_out_of_their_identity_token() {
        let text = format_message_mention(ALICE.into(), "Alice]\n\\[bad".into());
        assert_eq!(message_mentions(text.clone())[0].label, "@Alicebad");
        assert!(mentions_owner(&text, ALICE));
    }
}
