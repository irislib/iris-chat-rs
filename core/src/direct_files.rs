use serde::{Deserialize, Serialize};

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectFileTransferStatus {
    Offered,
    Connecting,
    Transferring,
    Completed,
    Declined,
    Cancelled,
    Failed,
    Unavailable,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DirectFileSnapshot {
    pub filename: String,
    pub size_bytes: u64,
    pub local_path: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DirectFileTransferSnapshot {
    pub id: String,
    pub files: Vec<DirectFileSnapshot>,
    pub status: DirectFileTransferStatus,
    /// The device sending the bytes, rather than the account authoring the message.
    pub is_sender: bool,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

// Shared by signed manifests, preparation, transport and chosen destinations.
pub(crate) fn valid_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
}

pub(crate) fn valid_transfer_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
