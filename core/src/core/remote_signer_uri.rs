use super::*;

pub(super) const PERMISSIONS: &str = "sign_event:37368";
const MAX_RELAYS: usize = 8;

pub(super) struct SignerConnection {
    pub(super) signer: PublicKey,
    pub(super) relays: Vec<RelayUrl>,
    pub(super) secret: String,
}

pub(super) fn parse_signer_connection(input: &str) -> anyhow::Result<SignerConnection> {
    anyhow::ensure!(input.len() <= 8192, "Invalid signer link.");
    let url = url::Url::parse(input.trim()).map_err(|_| anyhow::anyhow!("Invalid signer link."))?;
    anyhow::ensure!(
        url.scheme() == "bunker"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.path().is_empty()
            && url.fragment().is_none(),
        "Paste a signer link starting with bunker://."
    );
    let signer = PublicKey::from_hex(url.host_str().unwrap_or_default())
        .map_err(|_| anyhow::anyhow!("Invalid signer user ID."))?;
    let mut secret = None;
    let mut relays = Vec::new();
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "relay" => relays.push(value.into_owned()),
            "secret" => {
                anyhow::ensure!(
                    secret.is_none() && value.len() <= 1024,
                    "Invalid signer link."
                );
                secret = Some(value.into_owned());
            }
            _ => {}
        }
    }
    Ok(SignerConnection {
        signer,
        relays: validate_signer_relays(&relays)?,
        secret: secret.unwrap_or_default(),
    })
}

pub(super) fn validate_signer_relays(inputs: &[String]) -> anyhow::Result<Vec<RelayUrl>> {
    anyhow::ensure!(
        !inputs.is_empty() && inputs.len() <= MAX_RELAYS,
        "Invalid signer message servers."
    );
    inputs
        .iter()
        .map(|input| {
            let url = url::Url::parse(input)?;
            anyhow::ensure!(
                input.len() <= 2048
                    && matches!(url.scheme(), "ws" | "wss")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none(),
                "Invalid signer message server."
            );
            Ok(RelayUrl::parse(input)?)
        })
        .collect()
}

pub(super) fn client_connection_uri(keys: &Keys, relays: &[RelayUrl], secret: &str) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for relay in relays {
        query.append_pair("relay", relay.as_str());
    }
    query.append_pair("secret", secret);
    query.append_pair("perms", PERMISSIONS);
    query.append_pair("name", "Iris Chat");
    query.append_pair("url", "https://iris.to");
    format!(
        "nostrconnect://{}?{}",
        keys.public_key().to_hex(),
        query.finish()
    )
}

pub(super) fn safe_auth_url(input: &str) -> Option<String> {
    let url = url::Url::parse(input).ok()?;
    (input.len() <= 4096
        && matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}
