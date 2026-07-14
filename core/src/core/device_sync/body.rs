use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serializer};

pub(super) fn serialize<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&STANDARD.encode(value.as_bytes()))
}

pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;
    let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
    String::from_utf8(bytes).map_err(serde::de::Error::custom)
}
