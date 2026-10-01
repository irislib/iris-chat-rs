use super::*;
use nostr::{Event, EventBuilder, JsonUtil, Kind};
use serde::{Deserialize, Serialize};

pub(super) const PREFIX: &str = "iris-direct-file-v1:";
const OFFER_KIND: Kind = Kind::Custom(21111);
pub(super) const MAX_FILES: usize = 32;
pub(super) const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ManifestFile {
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Offer {
    pub id: String,
    pub token: String,
    pub owner: String,
    pub recipient: String,
    pub device: String,
    pub caption: String,
    pub expires_at_secs: u64,
    pub files: Vec<ManifestFile>,
}

impl Offer {
    pub(super) fn wire(&self, keys: &Keys) -> Result<String, String> {
        let content = serde_json::to_string(self).map_err(|e| e.to_string())?;
        let event = EventBuilder::new(OFFER_KIND, content)
            .sign_with_keys(keys)
            .map_err(|e| e.to_string())?;
        let wire = format!("{PREFIX}{}", event.as_json());
        if wire.len() > 32 * 1024 {
            return Err("The file offer is too large.".into());
        }
        Ok(wire)
    }

    pub(super) fn total(&self) -> u64 {
        self.files.iter().map(|f| f.size_bytes).sum()
    }

    pub(super) fn transport_files(&self, paths: &[String]) -> Vec<TransferFile> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| TransferFile {
                filename: f.filename.clone(),
                size_bytes: f.size_bytes,
                sha256: f.sha256.clone(),
                path: paths.get(i).map(PathBuf::from).unwrap_or_default(),
            })
            .collect()
    }
}

pub(super) fn parse(body: &str) -> Option<Offer> {
    let raw = body.strip_prefix(PREFIX)?;
    if raw.len() > 32 * 1024 {
        return None;
    }
    let event = Event::from_json(raw).ok()?;
    if event.kind != OFFER_KIND || event.verify().is_err() {
        return None;
    }
    let offer: Offer = serde_json::from_str(&event.content).ok()?;
    if offer.device != event.pubkey.to_hex()
        || !is_hex(&offer.id, 32)
        || !is_hex(&offer.token, 64)
        || !is_hex(&offer.owner, 64)
        || !is_hex(&offer.recipient, 64)
        || offer.caption.len() > 4096
        || offer.expires_at_secs > event.created_at.as_secs().saturating_add(24 * 60 * 60)
        || offer.expires_at_secs <= event.created_at.as_secs()
        || offer.files.is_empty()
        || offer.files.len() > MAX_FILES
        || offer.files.iter().any(|f| {
            f.filename.is_empty()
                || f.filename.len() > 240
                || f.filename
                    .chars()
                    .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
                || f.filename == "."
                || f.filename == ".."
                || f.size_bytes > MAX_FILE_BYTES
                || !is_hex(&f.sha256, 64)
        })
    {
        return None;
    }
    Some(offer)
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Record {
    pub chat_id: String,
    pub wire: String,
    pub offer: Offer,
    pub is_sender: bool,
    pub status: DirectFileTransferStatus,
    pub paths: Vec<String>,
    pub peer: Option<String>,
    pub transferred: u64,
    pub error: Option<String>,
}

pub(super) fn active(status: &DirectFileTransferStatus) -> bool {
    matches!(
        status,
        DirectFileTransferStatus::Offered
            | DirectFileTransferStatus::Connecting
            | DirectFileTransferStatus::Transferring
    )
}
