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
        if message.contains("session") && message.contains("not ready") {
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
