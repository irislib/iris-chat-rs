use std::fmt;

#[derive(Debug, Clone, uniffi::Error)]
pub enum NdrError {
    InvalidKey(String),
    InvalidEvent(String),
    CryptoFailure(String),
    StateMismatch(String),
    Serialization(String),
    InviteError(String),
    SessionNotReady(String),
}

impl fmt::Display for NdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NdrError::InvalidKey(message) => write!(f, "Invalid key: {message}"),
            NdrError::InvalidEvent(message) => write!(f, "Invalid event: {message}"),
            NdrError::CryptoFailure(message) => write!(f, "Crypto failure: {message}"),
            NdrError::StateMismatch(message) => write!(f, "State mismatch: {message}"),
            NdrError::Serialization(message) => write!(f, "Serialization error: {message}"),
            NdrError::InviteError(message) => write!(f, "Invite error: {message}"),
            NdrError::SessionNotReady(message) => write!(f, "Session not ready: {message}"),
        }
    }
}

impl std::error::Error for NdrError {}

impl From<serde_json::Error> for NdrError {
    fn from(error: serde_json::Error) -> Self {
        NdrError::Serialization(error.to_string())
    }
}

impl From<hex::FromHexError> for NdrError {
    fn from(error: hex::FromHexError) -> Self {
        NdrError::InvalidKey(error.to_string())
    }
}

impl From<iris_chat_protocol::StorageError> for NdrError {
    fn from(error: iris_chat_protocol::StorageError) -> Self {
        NdrError::Serialization(error.to_string())
    }
}

impl From<nostr::key::Error> for NdrError {
    fn from(error: nostr::key::Error) -> Self {
        NdrError::InvalidKey(error.to_string())
    }
}

impl From<nostr::event::Error> for NdrError {
    fn from(error: nostr::event::Error) -> Self {
        NdrError::InvalidEvent(error.to_string())
    }
}

impl From<anyhow::Error> for NdrError {
    fn from(error: anyhow::Error) -> Self {
        let message = error.to_string();
        if error.is::<iris_chat_protocol::PendingInboundAdmissionError>()
            || (message.contains("session") && message.contains("not ready"))
        {
            NdrError::SessionNotReady(message)
        } else if message.contains("invite") {
            NdrError::InviteError(message)
        } else if message.contains("decrypt") || message.contains("encrypt") {
            NdrError::CryptoFailure(message)
        } else if message.contains("key") {
            NdrError::InvalidKey(message)
        } else {
            NdrError::InvalidEvent(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_inbound_admission_is_retryable_without_reclassifying_invalid_events() {
        for context in [None, Some("invalid event received")] {
            let mut error = anyhow::Error::new(iris_chat_protocol::PendingInboundAdmissionError);
            if let Some(context) = context {
                error = error.context(context);
            }
            let expected = error.to_string();
            assert!(matches!(
                NdrError::from(error),
                NdrError::SessionNotReady(message) if message == expected
            ));
        }

        // The typed cause, including through context, selects retryability.
        // An ordinary invalid event with identical text must remain invalid.
        for message in [
            iris_chat_protocol::PendingInboundAdmissionError.to_string(),
            "malformed event".to_string(),
        ] {
            assert!(matches!(
                NdrError::from(anyhow::Error::msg(message.clone())),
                NdrError::InvalidEvent(detail) if detail == message
            ));
        }
    }
}
