//! Private invitation records: one standard CHK-encrypted hashtree leaf.
use hashtree_core::{decrypt_chk, nhash_decode, nhash_encode_full, sha256, NHashData};
use serde::{Deserialize, Serialize};

pub const MAX_INVITE_BYTES: usize = 16 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InviteConnection {
    pub inviter: String,
    pub ephemeral_key: String,
    pub shared_secret: String,
    pub owner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InviteRecord {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub nonce: String,
    pub invite: InviteConnection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
}
fn hex_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl InviteRecord {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.v == 1 && matches!(self.kind.as_str(), "personal" | "group"),
            "Invalid invite version or type"
        );
        anyhow::ensure!(
            self.name.encode_utf16().count() <= 100 && hex_key(&self.nonce),
            "Invalid invite metadata"
        );
        anyhow::ensure!(
            [
                &self.invite.inviter,
                &self.invite.ephemeral_key,
                &self.invite.shared_secret,
                &self.invite.owner
            ]
            .iter()
            .all(|v| hex_key(v)),
            "Invalid invite keys"
        );
        anyhow::ensure!(
            self.invite
                .device_id
                .as_ref()
                .is_none_or(|v| v.encode_utf16().count() <= 128)
                && self
                    .invite
                    .purpose
                    .as_deref()
                    .is_none_or(|v| matches!(v, "chat" | "private")),
            "Invalid invite connection"
        );
        anyhow::ensure!(
            self.expires_at
                .is_none_or(|v| v > 0 && v <= 9_007_199_254_740_991)
                && self.max_uses.is_none_or(|v| v > 0 && v <= 10000),
            "Invalid invite limits"
        );
        if self.kind == "group" {
            anyhow::ensure!(
                self.group_id
                    .as_ref()
                    .is_some_and(|v| !v.is_empty() && v.encode_utf16().count() <= 128)
                    && self.token.as_ref().is_some_and(|v| hex_key(v)),
                "Invalid group invite"
            );
        } else {
            anyhow::ensure!(
                self.group_id.is_none() && self.token.is_none() && self.max_uses.is_none(),
                "Invalid personal invite"
            );
        }
        Ok(())
    }
    pub fn legacy_url(&self) -> anyhow::Result<String> {
        let value = serde_json::to_string(&self.invite)?;
        let mut url = url::Url::parse("https://chat.iris.to/")?;
        url.set_fragment(Some(&value));
        Ok(url.to_string())
    }
}
/// Encode exactly one private CHK leaf, compatible with the web invite codec.
pub fn encode_record(record: &InviteRecord) -> anyhow::Result<(String, Vec<u8>)> {
    record.validate()?;
    let (ciphertext, key) = hashtree_core::encrypt_chk(&serde_json::to_vec(record)?)?;
    anyhow::ensure!(ciphertext.len() <= MAX_INVITE_BYTES, "Invite is too large");
    let reference = nhash_encode_full(&NHashData {
        hash: sha256(&ciphertext),
        decrypt_key: Some(key),
    })?;
    Ok((reference, ciphertext))
}

pub fn reference_from_input(input: &str) -> Option<&str> {
    let input = input.trim();
    let reference = input
        .rsplit_once('#')
        .map(|(_, value)| value)
        .unwrap_or(input)
        .trim_start_matches('/');
    reference.starts_with("nhash1").then_some(reference)
}
pub fn decode_record(reference: &str, ciphertext: &[u8]) -> anyhow::Result<InviteRecord> {
    anyhow::ensure!(
        reference.len() <= 160 && reference.starts_with("nhash1"),
        "Invalid private invite reference"
    );
    let cid = nhash_decode(reference)?;
    let key = cid
        .decrypt_key
        .ok_or_else(|| anyhow::anyhow!("Invite record must be private"))?;
    anyhow::ensure!(
        nhash_encode_full(&NHashData {
            hash: cid.hash,
            decrypt_key: Some(key)
        })? == reference,
        "Invalid private invite reference"
    );
    anyhow::ensure!(
        (16..=MAX_INVITE_BYTES).contains(&ciphertext.len()) && sha256(ciphertext) == cid.hash,
        "Invalid invite record"
    );
    let record: InviteRecord = serde_json::from_slice(&decrypt_chk(ciphertext, &key)?)?;
    record.validate()?;
    Ok(record)
}
pub async fn load_record(reference: &str) -> anyhow::Result<InviteRecord> {
    anyhow::ensure!(reference.len() <= 160, "Invalid private invite reference");
    let cid = nhash_decode(reference)?;
    anyhow::ensure!(cid.decrypt_key.is_some(), "Invite record must be private");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    // Storage locations come from local configuration, never from the invite.
    let mut servers = hashtree_config::Config::load_or_default()
        .blossom
        .all_read_servers();
    servers.truncate(4);
    for server in servers {
        let result = async {
            let mut response = client
                .get(format!("{server}/{}", hashtree_core::to_hex(&cid.hash)))
                .send()
                .await?
                .error_for_status()?;
            anyhow::ensure!(
                response
                    .content_length()
                    .is_none_or(|n| n <= MAX_INVITE_BYTES as u64),
                "Invite is too large"
            );
            let mut data = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                anyhow::ensure!(
                    data.len() + chunk.len() <= MAX_INVITE_BYTES,
                    "Invite is too large"
                );
                data.extend_from_slice(&chunk);
            }
            decode_record(reference, &data)
        }
        .await;
        if let Ok(record) = result {
            return Ok(record);
        }
    }
    anyhow::bail!("Could not load invite. Check your connection and try again.")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("private-invite-v1.json")).unwrap()
    }
    #[test]
    fn decrypts_standard_typescript_hashtree_invite() {
        let fixture = fixture();
        let reference = fixture["reference"].as_str().unwrap();
        let ciphertext = fixture["ciphertext"]
            .as_str()
            .unwrap()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let record = decode_record(reference, &ciphertext).unwrap();
        assert_eq!(serde_json::to_value(&record).unwrap(), fixture["record"]);
        assert!(record
            .legacy_url()
            .unwrap()
            .starts_with("https://chat.iris.to/#"));
        let (encrypted, key) =
            hashtree_core::encrypt_chk(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(encrypted, ciphertext);
        assert_eq!(
            nhash_encode_full(&NHashData {
                hash: sha256(&encrypted),
                decrypt_key: Some(key)
            })
            .unwrap(),
            reference
        );
    }
    #[test]
    fn rejects_public_and_tampered_records() {
        let fixture = fixture();
        let reference = fixture["reference"].as_str().unwrap();
        let mut ciphertext = fixture["ciphertext"]
            .as_str()
            .unwrap()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let public = hashtree_core::nhash_encode(&sha256(&ciphertext)).unwrap();
        assert!(decode_record(&public, &ciphertext).is_err());
        ciphertext[0] ^= 1;
        assert!(decode_record(reference, &ciphertext).is_err());
        assert!(decode_record(reference, &vec![0; MAX_INVITE_BYTES + 1]).is_err());
    }
    #[test]
    fn validates_record_limits_and_excludes_group_message_keys() {
        for (field, value) in [
            ("v", serde_json::json!(2)),
            ("maxUses", serde_json::json!(0)),
            ("secret", serde_json::json!("private")),
            ("token", serde_json::json!("")),
        ] {
            let mut invalid = fixture()["record"].clone();
            invalid[field] = value;
            assert!(serde_json::from_value::<InviteRecord>(invalid)
                .and_then(|r| r.validate().map_err(serde::de::Error::custom))
                .is_err());
        }
    }
}
