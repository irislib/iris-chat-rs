use super::*;

/// Presentation only: valid content references never expose a hash/key in a
/// notification or chat-list excerpt. The saved body and attachment stay intact.
pub(super) fn without_hash_references(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut copied = 0;
    while let Some(relative) = text[cursor..].find("nhash1") {
        let hash_start = cursor + relative;
        let mut end = hash_start;
        while bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) {
            end += 1;
        }
        cursor = end;
        if hashtree_core::nhash_decode(&text[hash_start..end]).is_err() {
            continue;
        }
        let mut start = hash_start;
        for prefix in ["htree://", "nhash://"] {
            if text[..start].ends_with(prefix) {
                start -= prefix.len();
                break;
            }
        }
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let mut label = "Attachment";
        if bytes.get(end) == Some(&b'/') {
            let filename_start = end + 1;
            end = filename_start;
            while bytes
                .get(end)
                .is_some_and(|byte| !byte.is_ascii_whitespace() && !b"<>\"')]}".contains(byte))
            {
                end += 1;
            }
            while end > filename_start && b".,;:!?".contains(&bytes[end - 1]) {
                end -= 1;
            }
            if let Some(attachment) = parse_file_link(&text[start..end]) {
                label = attachment_preview_label(&attachment);
            }
        }
        // Markdown/image wrappers are one attachment reference, not visible
        // punctuation around a hidden URL. Plain parentheses remain readable.
        if bytes.get(end) == Some(&b')') && text[..start].ends_with("](") {
            if let Some(open) = text[..start - 2].rfind('[') {
                let title = &text[open + 1..start - 2];
                if !title.contains(['\n', '[', ']']) {
                    start = if open > 0 && bytes[open - 1] == b'!' {
                        open - 1
                    } else {
                        open
                    };
                    end += 1;
                }
            }
        }
        if start < copied {
            continue;
        }
        output.push_str(&text[copied..start]);
        output.push_str(label);
        copied = end;
        cursor = end;
    }
    output.push_str(&text[copied..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    const HASH: &str = "nhash1qqsqzqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqge7dr00";
    #[test]
    fn nhash_preview_validates_checksum_and_preserves_captions_and_wrappers() {
        assert!(hashtree_core::nhash_decode(HASH).is_ok());
        for (input, expected) in [
            (HASH.to_string(), "Attachment"),
            (format!("Caption {HASH}"), "Caption Attachment"),
            (format!("({HASH})"), "(Attachment)"),
            (
                format!("htree://{HASH} nhash://{HASH}"),
                "Attachment Attachment",
            ),
            (format!("[Download]({HASH})"), "Attachment"),
            (format!("![Image](htree://{HASH}/photo.png)"), "Photo"),
            (format!("Caption {HASH}/clip.mp4!"), "Caption Video!"),
            (
                "A literal nhash1bad is ordinary text".into(),
                "A literal nhash1bad is ordinary text",
            ),
        ] {
            assert_eq!(without_hash_references(&input), expected, "{input}");
        }
        for input in [
            format!("prefix{HASH}"),
            format!("é{HASH}"),
            format!("_{HASH}"),
        ] {
            assert_eq!(without_hash_references(&input), input);
        }
        assert_eq!(
            without_hash_references(&format!("{HASH}, {HASH}!")),
            "Attachment, Attachment!"
        );
        let invalid = format!("{}x", &HASH[..HASH.len() - 1]);
        assert_eq!(without_hash_references(&invalid), invalid);
    }
    #[test]
    fn nhash_preview_sanitizes_reply_and_forward_bodies_without_rewriting_content() {
        let body = format!("↩ Alex: Earlier {HASH}\n\nCaption {HASH}");
        assert_eq!(chat_message_body_preview(&body), "Caption Attachment");
        assert!(body.contains(HASH));
        assert_eq!(
            chat_message_body_preview(&format!("Forwarded:\n\n{HASH}")),
            "Forwarded:\n\nAttachment"
        );
    }
}
