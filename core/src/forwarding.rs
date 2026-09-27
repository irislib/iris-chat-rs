//! Forwarded content stays readable in older clients and notification previews.
//! This is a sender-provided heading, not verified original-author attribution.

const FORWARDED_HEADING: &str = "Forwarded:\n\n";

/// Mark a forward without including the source chat, author or message ID.
/// Keep the heading in the encrypted body so every renderer can display it.
#[uniffi::export]
pub fn format_forwarded_message(text: String) -> String {
    let text = text.trim();
    // Attachment extraction trims an attachment-only forward to "Forwarded:";
    // native forward pickers append its attachment links on the next line.
    if text.is_empty() || text == "Forwarded:" || text.starts_with("Forwarded:\n") {
        return text.to_string();
    }
    format!("{FORWARDED_HEADING}{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarding_keeps_content_and_marks_it_once() {
        for body in [
            "Hello",
            "A caption\nhtree://nhash1example/photo.jpg",
            "😀",
            "Line one\n\nLine two",
        ] {
            let forwarded = format_forwarded_message(body.into());
            assert_eq!(forwarded, format!("Forwarded:\n\n{body}"));
            assert_eq!(format_forwarded_message(forwarded.clone()), forwarded);
        }
        assert_eq!(format_forwarded_message(" \n ".into()), "");
        assert_eq!(
            format_forwarded_message("Forwarded:\nhtree://nhash1example/photo.jpg".into()),
            "Forwarded:\nhtree://nhash1example/photo.jpg"
        );
        assert_eq!(
            format_forwarded_message("  Hello\n".into()),
            "Forwarded:\n\nHello"
        );
    }
}
