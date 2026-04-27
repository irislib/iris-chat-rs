use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileLinkMatch {
    start: usize,
    end: usize,
    attachment: MessageAttachmentSnapshot,
}

pub(super) fn extract_message_attachments(text: &str) -> (String, Vec<MessageAttachmentSnapshot>) {
    let matches = find_file_links(text);
    if matches.is_empty() {
        return (text.trim().to_string(), Vec::new());
    }

    let mut body = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut attachments = Vec::with_capacity(matches.len());

    for found in matches {
        body.push_str(&text[last..found.start]);
        last = found.end;
        attachments.push(found.attachment);
    }
    body.push_str(&text[last..]);

    (body.trim().to_string(), attachments)
}

pub(super) fn message_preview(message: &ChatMessageSnapshot) -> String {
    if !message.body.is_empty() {
        return message.body.clone();
    }
    match message.attachments.as_slice() {
        [] => String::new(),
        [attachment] => attachment_preview_label(attachment).to_string(),
        attachments => {
            if attachments.iter().all(|a| a.is_image) {
                format!("{} photos", attachments.len())
            } else {
                format!("{} attachments", attachments.len())
            }
        }
    }
}

fn attachment_preview_label(attachment: &MessageAttachmentSnapshot) -> &'static str {
    if attachment.is_image {
        "Photo"
    } else if attachment.is_video {
        "Video"
    } else if attachment.is_audio {
        "Audio"
    } else {
        "Attachment"
    }
}

pub(super) fn format_attachment_links_message(
    caption: &str,
    attachments: &[(String, String)],
) -> String {
    let caption = caption.trim();
    let file_links = attachments
        .iter()
        .map(|(nhash, filename)| format_file_link(nhash, filename))
        .collect::<Vec<_>>();
    match (caption.is_empty(), file_links.is_empty()) {
        (true, _) => file_links.join("\n"),
        (_, true) => caption.to_string(),
        (false, false) => format!("{caption}\n{}", file_links.join("\n")),
    }
}

pub(super) fn format_file_link(nhash: &str, filename: &str) -> String {
    format!("{}/{}", nhash.trim(), percent_encode_filename(filename))
}

fn find_file_links(text: &str) -> Vec<FileLinkMatch> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < text.len() {
        let Some(relative) = text[i..].find("nhash1") else {
            break;
        };
        let nhash_start = i + relative;
        let mut match_start = nhash_start;

        if nhash_start >= "htree://".len()
            && &text[nhash_start - "htree://".len()..nhash_start] == "htree://"
        {
            match_start = nhash_start - "htree://".len();
        } else if nhash_start >= "nhash://".len()
            && &text[nhash_start - "nhash://".len()..nhash_start] == "nhash://"
        {
            match_start = nhash_start - "nhash://".len();
        }

        let mut nhash_end = nhash_start;
        while nhash_end < bytes.len() {
            let byte = bytes[nhash_end];
            if byte == b'/' {
                break;
            }
            if !byte.is_ascii_alphanumeric() {
                break;
            }
            nhash_end += 1;
        }

        if nhash_end >= bytes.len() || bytes[nhash_end] != b'/' {
            i = nhash_start + "nhash1".len();
            continue;
        }

        let file_start = nhash_end + 1;
        let mut file_end = file_start;
        while file_end < bytes.len() && !bytes[file_end].is_ascii_whitespace() {
            file_end += 1;
        }
        if file_end == file_start {
            i = file_start;
            continue;
        }

        if let Some(attachment) = parse_file_link(&text[match_start..file_end]) {
            out.push(FileLinkMatch {
                start: match_start,
                end: file_end,
                attachment,
            });
        }
        i = file_end;
    }

    out
}

fn parse_file_link(link: &str) -> Option<MessageAttachmentSnapshot> {
    let cleaned = link
        .trim()
        .strip_prefix("htree://")
        .or_else(|| link.trim().strip_prefix("nhash://"))
        .unwrap_or_else(|| link.trim());
    let (nhash, filename_encoded) = cleaned.split_once('/')?;
    let nhash = nhash.trim();
    if !is_valid_nhash(nhash) {
        return None;
    }
    let filename_encoded = filename_encoded.trim();
    if filename_encoded.is_empty() {
        return None;
    }

    let filename = percent_decode(filename_encoded);
    Some(MessageAttachmentSnapshot {
        nhash: nhash.to_string(),
        filename,
        filename_encoded: filename_encoded.to_string(),
        htree_url: format!("htree://{nhash}/{filename_encoded}"),
        is_image: has_extension(
            filename_encoded,
            &["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "avif"],
        ),
        is_video: has_extension(filename_encoded, &["mp4", "webm", "mov", "avi", "mkv"]),
        is_audio: has_extension(
            filename_encoded,
            &["mp3", "wav", "ogg", "flac", "m4a", "aac"],
        ),
    })
}

fn is_valid_nhash(nhash: &str) -> bool {
    nhash.to_ascii_lowercase().starts_with("nhash1")
        && nhash.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn has_extension(filename: &str, extensions: &[&str]) -> bool {
    let decoded = percent_decode(filename);
    let Some((_, extension)) = decoded.rsplit_once('.') else {
        return false;
    };
    extensions
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push((high << 4) | low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

fn percent_encode_filename(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_nhash_attachment_and_strips_visible_body() {
        let (body, attachments) = extract_message_attachments("here\nnhash1abc123/photo%201.png\n");

        assert_eq!(body, "here");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].nhash, "nhash1abc123");
        assert_eq!(attachments[0].filename, "photo 1.png");
        assert!(attachments[0].is_image);
    }

    #[test]
    fn accepts_htree_and_nhash_wrappers() {
        let (_, attachments) = extract_message_attachments(
            "htree://nhash1abc123/clip.mp4 nhash://nhash1def456/song.m4a",
        );

        assert_eq!(attachments.len(), 2);
        assert!(attachments[0].is_video);
        assert!(attachments[1].is_audio);
        assert_eq!(attachments[0].htree_url, "htree://nhash1abc123/clip.mp4");
    }

    #[test]
    fn ignores_invalid_links() {
        let (body, attachments) = extract_message_attachments("npub1abc/file.png nhash1bad");

        assert_eq!(body, "npub1abc/file.png nhash1bad");
        assert!(attachments.is_empty());
    }

    #[test]
    fn formats_attachment_messages_with_encoded_filename() {
        assert_eq!(
            format_attachment_links_message(
                "hello",
                &[("nhash1abc123".to_string(), "photo 1.png".to_string())],
            ),
            "hello\nnhash1abc123/photo%201.png"
        );
        assert_eq!(
            format_attachment_links_message(
                "",
                &[("nhash1abc123".to_string(), "m\u{00F6}te.txt".to_string())],
            ),
            "nhash1abc123/m%C3%B6te.txt"
        );
        assert_eq!(
            format_attachment_links_message(
                "album",
                &[
                    ("nhash1abc123".to_string(), "one.png".to_string()),
                    ("nhash1def456".to_string(), "two final.png".to_string()),
                ],
            ),
            "album\nnhash1abc123/one.png\nnhash1def456/two%20final.png"
        );
    }
}
