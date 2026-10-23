use super::{StorageError, StorageResult};
use nostr::hashes::{sha256, Hash};
use std::collections::HashMap;
use std::sync::Arc;

/// Immutable local checkpoint bytes. Its content identity is computed once,
/// never published, and relies on SHA-256 collision resistance. Keeping the
/// identity and bytes private prevents a caller from reusing an ID for new data.
pub struct StorageCheckpointBody {
    pub(super) id: [u8; 32],
    json: Arc<serde_json::value::RawValue>,
}

impl StorageCheckpointBody {
    pub fn new(json: Arc<serde_json::value::RawValue>) -> Self {
        Self {
            id: sha256::Hash::hash(json.get().as_bytes()).to_byte_array(),
            json,
        }
    }
    pub fn json(&self) -> &str {
        self.json.get()
    }
}

impl serde::Serialize for StorageCheckpointBody {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(self.json.as_ref(), serializer)
    }
}

pub(super) enum PartBytes {
    Inline(String),
    Immutable(Arc<StorageCheckpointBody>),
}

impl PartBytes {
    pub(super) fn bytes(&self) -> &[u8] {
        match self {
            Self::Inline(value) => value.as_bytes(),
            Self::Immutable(body) => body.json().as_bytes(),
        }
    }
}

pub(super) struct CheckpointPart {
    pub key: Vec<u8>,
    pub payload_key: Vec<u8>,
    pub bytes: PartBytes,
    pub prefix: String,
    pub suffix: String,
    pub padding: usize,
}

impl CheckpointPart {
    pub(super) fn len(&self) -> usize {
        self.prefix
            .len()
            .saturating_add(self.bytes.bytes().len())
            .saturating_add(self.suffix.len())
            .saturating_add(self.padding)
    }
}

/// A lossless description of a logical String. SQLite can store its immutable
/// records independently; other adapters materialize the same bytes normally.
pub struct StorageCheckpoint {
    pub(super) parts: Vec<CheckpointPart>,
    occurrences: HashMap<(u8, [u8; 32]), u64>,
    bytes: usize,
}

impl Default for StorageCheckpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageCheckpoint {
    pub fn new() -> Self {
        Self {
            parts: Vec::new(),
            occurrences: HashMap::new(),
            bytes: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.bytes
    }
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }

    pub fn push_inline(&mut self, name: &str, text: String, padding: usize) {
        let mut key = vec![0];
        key.extend_from_slice(name.as_bytes());
        let part = CheckpointPart {
            payload_key: key.clone(),
            key,
            bytes: PartBytes::Inline(text),
            prefix: String::new(),
            suffix: String::new(),
            padding,
        };
        self.bytes = self.bytes.saturating_add(part.len());
        self.parts.push(part);
    }

    pub fn push_record(
        &mut self,
        collection: u8,
        body: Arc<StorageCheckpointBody>,
        prefix: String,
        suffix: String,
        padding: usize,
    ) {
        let occurrence = self.occurrences.entry((collection, body.id)).or_default();
        let mut key = vec![1, collection];
        key.extend_from_slice(&body.id);
        key.extend_from_slice(&occurrence.to_be_bytes());
        *occurrence += 1;
        let mut payload_key = vec![1];
        payload_key.extend_from_slice(&body.id);
        let part = CheckpointPart {
            key,
            payload_key,
            bytes: PartBytes::Immutable(body),
            prefix,
            suffix,
            padding,
        };
        self.bytes = self.bytes.saturating_add(part.len());
        self.parts.push(part);
    }

    pub fn pad_records(&mut self, page: usize, minimum_record: usize) {
        let page = page.max(1);
        for part in &mut self.parts {
            if matches!(&part.bytes, PartBytes::Immutable(_)) && part.len() >= minimum_record {
                let padding = part
                    .len()
                    .div_ceil(page)
                    .saturating_mul(page)
                    .saturating_sub(part.len());
                part.padding = part.padding.saturating_add(padding);
                self.bytes = self.bytes.saturating_add(padding);
            }
        }
    }

    pub fn materialize(&self) -> StorageResult<String> {
        self.validate()?;
        let mut result = String::new();
        result
            .try_reserve_exact(self.bytes)
            .map_err(|error| StorageError::new(error.to_string()))?;
        for part in &self.parts {
            result.push_str(&part.prefix);
            // Both variants originate in String or validated RawValue.
            result.push_str(
                std::str::from_utf8(part.bytes.bytes())
                    .map_err(|_| StorageError::new("invalid checkpoint UTF-8"))?,
            );
            result.push_str(&part.suffix);
            append_padding(&mut result, part.padding);
        }
        Ok(result)
    }

    pub(super) fn validate(&self) -> StorageResult<()> {
        let total = self.parts.iter().try_fold(0usize, |total, part| {
            part.prefix
                .len()
                .checked_add(part.bytes.bytes().len())
                .and_then(|n| n.checked_add(part.suffix.len()))
                .and_then(|n| n.checked_add(part.padding))
                .and_then(|n| total.checked_add(n))
        });
        if total != Some(self.bytes) || i64::try_from(self.bytes).is_err() {
            return Err(StorageError::new("invalid checkpoint size"));
        }
        Ok(())
    }
}

pub(super) fn append_padding(output: &mut String, mut bytes: usize) {
    const SPACES: &str = "                                                                ";
    while bytes >= SPACES.len() {
        output.push_str(SPACES);
        bytes -= SPACES.len();
    }
    output.push_str(&SPACES[..bytes]);
}
