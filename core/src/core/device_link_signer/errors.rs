// Never return arbitrary parser, storage, or network errors to another device:
// they may contain request contents or local paths. Keep actionable failures.
pub(in crate::core) fn safe_error(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        "Device approval expired." => "Device approval expired. Try again.",
        "Device approval cancelled." => "Device approval cancelled.",
        "Primary device required." | "Approving device is not authorized." => {
            "Use your primary device to approve this link."
        }
        "Device list changed." | "Device list changed. Try again." => {
            "Device list changed. Try again."
        }
        "Conflicting device lists." | "Conflicting device lists. Try again later." => {
            "Conflicting device lists. Try again later."
        }
        "Could not check all message servers. Try again." | "No message servers available." => {
            "Could not check message servers. Try again."
        }
        "Too many linked devices." => "Too many linked devices.",
        "Device list is too large." => "Device list is too large.",
        "Device link cannot change existing devices." => "Device list does not match. Try again.",
        "Invalid device authorization."
        | "Invalid new device."
        | "A device link must add one device."
        | "Invalid device profile." => "Invalid device link. Try again.",
        "This link already approved a device." => "This link already approved a device.",
        "Could not save device link." => "Could not save device link. Try again.",
        "Unsupported method." => "Unsupported method.",
        // Errors returned by the core cross an internal channel before reaching
        // the transport. Preserve only the same fixed, safe vocabulary.
        "Device approval expired. Try again." => "Device approval expired. Try again.",
        "Use your primary device to approve this link." => {
            "Use your primary device to approve this link."
        }
        "Could not check message servers. Try again." => {
            "Could not check message servers. Try again."
        }
        "Device list does not match. Try again." => "Device list does not match. Try again.",
        "Invalid device link. Try again." => "Invalid device link. Try again.",
        "Could not save device link. Try again." => "Could not save device link. Try again.",
        _ => "Could not approve this link. Try again.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_errors_preserve_safe_reasons_without_exposing_untrusted_details() {
        for message in [
            "Device list changed. Try again.",
            "Too many linked devices.",
        ] {
            assert_eq!(safe_error(&anyhow::anyhow!(message)), message);
        }
        for message in [
            "malformed request containing secret",
            "database /private/account failed",
        ] {
            assert_eq!(
                safe_error(&anyhow::anyhow!(message)),
                "Could not approve this link. Try again."
            );
        }
    }
}
