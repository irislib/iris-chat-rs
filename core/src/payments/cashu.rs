use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use serde::Deserialize;
use url::Url;

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct CashuMintAmount {
    pub url: String,
    pub name: String,
    pub amount: u64,
}

/// Unverified, sender-provided metadata. This does not establish that the mint
/// issued the proofs, that spending conditions are met, or that funds remain.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct CashuTokenMetadata {
    pub amount: u64,
    pub unit: String,
    pub memo: Option<String>,
    pub mints: Vec<CashuMintAmount>,
    pub proof_count: u32,
}

#[uniffi::export]
pub fn parse_cashu_token(text: String) -> Option<CashuTokenMetadata> {
    if text.len() > super::MAX_TEXT_BYTES {
        return None;
    }
    let raw = super::strip_scheme(text.trim(), "cashu:");
    let payload = raw.get(6..)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    let mut result = match raw.get(..6)? {
        "cashuA" => parse_v3(&decoded)?,
        "cashuB" => parse_v4(&decoded)?,
        _ => return None,
    };
    if result.unit.is_empty()
        || result.unit.len() > 12
        || !result.unit.bytes().all(|b| b.is_ascii_alphabetic())
        || result.memo.as_ref().is_some_and(|m| m.len() > 1024)
        || result.proof_count == 0
        || result.proof_count > 256
    {
        return None;
    }
    for mint in &result.mints {
        result.amount = result.amount.checked_add(mint.amount)?;
    }
    (result.amount > 0).then_some(result)
}

fn mint_amount(raw: &str, amount: u64) -> Option<CashuMintAmount> {
    if raw.len() > 1024 || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    // HTTP mints (including local test mints) are metadata only; no fetch occurs.
    let name = url.host_str()?.to_owned();
    Some(CashuMintAmount {
        url: url.as_str().trim_end_matches('/').to_owned(),
        name,
        amount,
    })
}

fn keyset(bytes: &[u8]) -> bool {
    matches!(bytes.len(), 8 | 33)
}
fn signature(bytes: &[u8]) -> bool {
    bytes.len() == 33 && matches!(bytes.first(), Some(2 | 3))
}
fn hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = char::from(*pair.first()?).to_digit(16)?;
            let low = char::from(*pair.get(1)?).to_digit(16)?;
            Some((high * 16 + low) as u8)
        })
        .collect()
}

#[derive(Deserialize)]
struct V3 {
    token: Vec<V3Mint>,
    unit: Option<String>,
    memo: Option<String>,
}
#[derive(Deserialize)]
struct V3Mint {
    mint: String,
    proofs: Vec<V3Proof>,
}
#[derive(Deserialize)]
struct V3Proof {
    amount: u64,
    id: String,
    secret: String,
    #[serde(rename = "C")]
    signature: String,
}

fn parse_v3(bytes: &[u8]) -> Option<CashuTokenMetadata> {
    let token: V3 = serde_json::from_slice(bytes).ok()?;
    let mut result = CashuTokenMetadata {
        amount: 0,
        unit: token.unit.unwrap_or_else(|| "sat".to_owned()),
        memo: token.memo,
        mints: vec![],
        proof_count: 0,
    };
    for mint in token.token {
        if mint.proofs.is_empty() {
            return None;
        }
        let mut amount = 0u64;
        for proof in mint.proofs {
            if proof.amount == 0
                || proof.secret.is_empty()
                || !keyset(&hex(&proof.id)?)
                || !signature(&hex(&proof.signature)?)
            {
                return None;
            }
            amount = amount.checked_add(proof.amount)?;
            result.proof_count = result.proof_count.checked_add(1)?;
        }
        result.mints.push(mint_amount(&mint.mint, amount)?);
    }
    Some(result)
}

// Require CBOR byte strings, not arrays/strings that merely resemble them.
struct Bytes(Vec<u8>);
impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Bytes;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a byte string")
            }
            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Bytes, E> {
                Ok(Bytes(value.to_vec()))
            }
            fn visit_byte_buf<E: serde::de::Error>(self, value: Vec<u8>) -> Result<Bytes, E> {
                Ok(Bytes(value))
            }
        }
        deserializer.deserialize_byte_buf(Visitor)
    }
}
#[derive(Deserialize)]
struct V4 {
    m: String,
    u: String,
    d: Option<String>,
    t: Vec<V4Keyset>,
}
#[derive(Deserialize)]
struct V4Keyset {
    i: Bytes,
    p: Vec<V4Proof>,
}
#[derive(Deserialize)]
struct V4Proof {
    a: u64,
    s: String,
    c: Bytes,
}

fn parse_v4(bytes: &[u8]) -> Option<CashuTokenMetadata> {
    let mut reader = bytes;
    let token: V4 = ciborium::de::from_reader_with_recursion_limit(&mut reader, 16).ok()?;
    if !reader.is_empty() {
        return None;
    }
    let mut amount = 0u64;
    let mut proof_count = 0u32;
    for group in token.t {
        if !keyset(&group.i.0) || group.p.is_empty() {
            return None;
        }
        for proof in group.p {
            if proof.a == 0 || proof.s.is_empty() || !signature(&proof.c.0) {
                return None;
            }
            amount = amount.checked_add(proof.a)?;
            proof_count = proof_count.checked_add(1)?;
        }
    }
    Some(CashuTokenMetadata {
        amount: 0,
        unit: token.u,
        memo: token.d,
        mints: vec![mint_amount(&token.m, amount)?],
        proof_count,
    })
}
