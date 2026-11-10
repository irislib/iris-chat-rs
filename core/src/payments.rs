//! Local payment presentation only. Never contacts a mint, pays an invoice, or
//! redeems a token. The original encrypted message remains the source of truth.
mod cashu;
pub use cashu::{parse_cashu_token, CashuMintAmount, CashuTokenMetadata};

use lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescriptionRef, Currency};

const MAX_TEXT_BYTES: usize = 16_000;
const MAX_PAYMENTS: usize = 4;

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct LightningInvoiceMetadata {
    pub amount_msats: Option<u64>,
    pub description: Option<String>,
    pub network: String,
    pub expires_at_secs: u64,
}

/// A compact link with UTF-16 offsets, like MessageMention. The URI contains
/// bearer/payment data: do not log it or send it to a link-preview service.
#[derive(uniffi::Record, Clone)]
pub struct MessagePaymentLink {
    pub start: u32,
    pub length: u32,
    pub label: String,
    pub uri: String,
    pub cashu: Option<CashuTokenMetadata>,
    pub lightning: Option<LightningInvoiceMetadata>,
}

#[uniffi::export]
pub fn parse_lightning_invoice(text: String) -> Option<LightningInvoiceMetadata> {
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    let raw = strip_scheme(text.trim(), "lightning:");
    // The maintained BOLT11 parser checks the checksum, signature and required
    // fields. An expired invoice is still readable; the wallet decides payment.
    let invoice: Bolt11Invoice = raw.parse().ok()?;
    let network = match invoice.currency() {
        Currency::Bitcoin => "Bitcoin",
        Currency::BitcoinTestnet => "Testnet",
        Currency::Regtest => "Regtest",
        Currency::Signet => "Signet",
        _ => return None,
    };
    Some(LightningInvoiceMetadata {
        amount_msats: invoice.amount_milli_satoshis(),
        description: match invoice.description() {
            Bolt11InvoiceDescriptionRef::Direct(value) => Some(value.to_string()),
            Bolt11InvoiceDescriptionRef::Hash(_) => None,
        },
        network: network.to_owned(),
        expires_at_secs: invoice.expires_at()?.as_secs(),
    })
}

/// Recognizes standalone payments, including optional URI schemes. Invalid,
/// embedded-URL and oversized candidates remain ordinary selectable text.
#[uniffi::export]
pub fn message_payment_links(text: String) -> Vec<MessagePaymentLink> {
    if text.len() > MAX_TEXT_BYTES {
        return vec![];
    }
    let mut links = Vec::new();
    let mut offset = 0;
    for part in text.split_inclusive(char::is_whitespace) {
        let trimmed = part
            .trim_end()
            .trim_end_matches(['.', ',', ';', '!', '?', ')', ']', '}', '>', '\'', '"']);
        let candidate = trimmed.trim_start_matches(['(', '[', '{', '<', '\'', '"']);
        let start = offset + trimmed.len() - candidate.len();
        offset += part.len();
        let cashu_raw = strip_scheme(candidate, "cashu:");
        let lightning_raw = strip_scheme(candidate, "lightning:");
        let (label, uri, cashu, lightning) =
            if cashu_raw.starts_with("cashuA") || cashu_raw.starts_with("cashuB") {
                let Some(token) = parse_cashu_token(candidate.to_owned()) else {
                    continue;
                };
                let unit = if token.unit == "sat" {
                    "sats"
                } else {
                    &token.unit
                };
                let mint = if token.mints.len() == 1 {
                    token
                        .mints
                        .first()
                        .map(|m| m.name.clone())
                        .unwrap_or_default()
                } else {
                    format!("{} mints", token.mints.len())
                };
                let test_money = if token
                    .mints
                    .iter()
                    .any(|mint| mint.url == "https://testnut.cashu.space")
                {
                    " · Test money"
                } else {
                    ""
                };
                (
                    format!(
                        "Cashu · {} {unit} · {mint}{test_money} · Not checked",
                        token.amount
                    ),
                    format!("cashu:{cashu_raw}"),
                    Some(token),
                    None,
                )
            } else if lightning_raw
                .get(..2)
                .is_some_and(|p| p.eq_ignore_ascii_case("ln"))
            {
                let Some(invoice) = parse_lightning_invoice(candidate.to_owned()) else {
                    continue;
                };
                let amount = match invoice.amount_msats {
                    Some(msats) if msats % 1000 == 0 => format!("{} sats", msats / 1000),
                    Some(msats) => format!("{}.{:03} sats", msats / 1000, msats % 1000),
                    None => "Choose amount".to_owned(),
                };
                let network = if invoice.network == "Bitcoin" {
                    String::new()
                } else {
                    format!(" · {}", invoice.network)
                };
                (
                    format!("Lightning · {amount}{network}"),
                    format!("lightning:{lightning_raw}"),
                    None,
                    Some(invoice),
                )
            } else {
                continue;
            };
        links.push(MessagePaymentLink {
            start: text.get(..start).unwrap_or_default().encode_utf16().count() as u32,
            length: candidate.encode_utf16().count() as u32,
            label,
            uri,
            cashu,
            lightning,
        });
        if links.len() == MAX_PAYMENTS {
            break;
        }
    }
    links
}

fn strip_scheme<'a>(text: &'a str, scheme: &str) -> &'a str {
    if text
        .get(..scheme.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(scheme))
    {
        text.get(scheme.len()..).unwrap_or_default()
    } else {
        text
    }
}

#[cfg(test)]
mod tests;
