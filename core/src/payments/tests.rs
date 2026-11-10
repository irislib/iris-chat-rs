use super::*;
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use serde_json::json;

// Published, non-spendable specification examples. Never use real funds here.
const V3: &str = include_str!("fixtures/cashu-A-0.txt");
const V4: &str = include_str!("fixtures/cashu-B-0.txt");
const INVOICE: &str = include_str!("fixtures/lightning-1.txt");
const AMOUNTLESS: &str = include_str!("fixtures/lightning-0.txt");

fn v3(value: serde_json::Value) -> String {
    format!(
        "cashuA{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
    )
}
fn v3_json() -> serde_json::Value {
    serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(V3.trim().strip_prefix("cashuA").unwrap())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn cashu_spec_vectors_and_uri_padding() {
    for raw in [V3.trim(), V4.trim()] {
        let info = parse_cashu_token(raw.to_owned()).unwrap();
        assert!(info.amount > 0);
        assert_eq!(info.unit, "sat");
        assert!(!info.mints.is_empty());
        assert!(info.proof_count > 0);
        assert_eq!(
            parse_cashu_token(format!("  cashu:{raw}\n")),
            Some(info.clone())
        );
        let bytes = URL_SAFE_NO_PAD.decode(&raw[6..]).unwrap();
        assert_eq!(
            parse_cashu_token(format!("{}{}", &raw[..6], URL_SAFE.encode(bytes))),
            Some(info)
        );
    }
    let info = parse_cashu_token(V3.trim().into()).unwrap();
    assert_eq!(info.amount, 10);
    assert_eq!(info.memo.as_deref(), Some("Thank you."));
    assert_eq!(info.mints[0].name, "8333.space");
}

#[test]
fn cashu_multimint_and_missing_unit() {
    let mut value = v3_json();
    let mut second = value["token"][0].clone();
    second["mint"] = json!("http://localhost:3338/");
    value["token"].as_array_mut().unwrap().push(second);
    value.as_object_mut().unwrap().remove("unit");
    let info = parse_cashu_token(v3(value)).unwrap();
    assert_eq!(info.amount, 20);
    assert_eq!(info.mints.len(), 2);
    assert_eq!(info.unit, "sat");
    assert_eq!(info.mints[1].url, "http://localhost:3338");
}

#[test]
fn known_test_mint_is_identified_without_contacting_it() {
    let mut value = v3_json();
    value["token"][0]["mint"] = json!("https://testnut.cashu.space/");
    let links = message_payment_links(v3(value));
    assert_eq!(
        links[0].label,
        "Cashu · 10 sats · testnut.cashu.space · Test money · Not checked"
    );
}

#[test]
fn cashu_rejects_malformed_proofs_and_overflow() {
    for (field, value) in [
        ("amount", json!(-1)),
        ("amount", json!(0)),
        ("amount", json!(1.5)),
        ("amount", json!(u64::MAX)),
        ("secret", json!("")),
        ("id", json!("bad")),
        ("C", json!("00")),
    ] {
        let mut token = v3_json();
        token["token"][0]["proofs"][0][field] = value;
        assert!(parse_cashu_token(v3(token)).is_none(), "{field}");
    }
    for mint in [
        "javascript:alert(1)",
        "https://user:pass@example.com",
        "https://example.com/?token=x",
        "https://example.com/#x",
        "https://exa\nmple.com",
    ] {
        let mut token = v3_json();
        token["token"][0]["mint"] = json!(mint);
        assert!(parse_cashu_token(v3(token)).is_none());
    }
    for text in [
        "cashuBbad",
        "cashuAeyJ4IjoxfQ",
        "cashuCanything",
        "cashuA",
        "cashuaABC",
    ] {
        assert!(parse_cashu_token(text.into()).is_none());
    }
}

#[test]
fn cashu_cbor_rejects_trailing_data_and_excessive_nesting() {
    let mut bytes = URL_SAFE_NO_PAD.decode(&V4.trim()[6..]).unwrap();
    bytes.push(0);
    assert!(parse_cashu_token(format!("cashuB{}", URL_SAFE_NO_PAD.encode(bytes))).is_none());
    let mut bytes = vec![0x81; 100];
    bytes.push(0);
    assert!(parse_cashu_token(format!("cashuB{}", URL_SAFE_NO_PAD.encode(bytes))).is_none());
}

#[test]
fn bolt11_spec_vectors_amountless_uppercase_and_checksum() {
    let invoice = parse_lightning_invoice(INVOICE.trim().into()).unwrap();
    assert_eq!(invoice.amount_msats, Some(250_000_000));
    assert_eq!(invoice.description.as_deref(), Some("1 cup coffee"));
    assert_eq!(invoice.network, "Bitcoin");
    assert_eq!(invoice.expires_at_secs, 1496314718);
    assert_eq!(
        parse_lightning_invoice(format!("LIGHTNING:{}", INVOICE.trim().to_uppercase())),
        Some(invoice)
    );
    assert_eq!(
        parse_lightning_invoice(AMOUNTLESS.trim().into())
            .unwrap()
            .amount_msats,
        None
    );
    let mut broken = INVOICE.trim().to_owned();
    broken.pop();
    broken.push('q');
    assert!(parse_lightning_invoice(broken).is_none());
    assert!(parse_lightning_invoice("lnbc123bad".into()).is_none());
}

#[test]
fn message_links_preserve_unicode_offsets_punctuation_and_bearer_text() {
    let token = V3.trim();
    let invoice = INVOICE.trim();
    let body = format!("💜 Gift: (cashu:{token}), pay {invoice}! Thanks");
    let links = message_payment_links(body.clone());
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].start, 10);
    assert_eq!(links[0].length as usize, token.len() + 6);
    assert_eq!(links[0].uri, format!("cashu:{token}"));
    assert_eq!(links[0].label, "Cashu · 10 sats · 8333.space · Not checked");
    assert_eq!(links[1].label, "Lightning · 250000 sats");
    assert_eq!(links[1].uri, format!("lightning:{invoice}"));
    for link in links {
        let visible = String::from_utf16(
            &body
                .encode_utf16()
                .skip(link.start as usize)
                .take(link.length as usize)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(visible == invoice || visible == format!("cashu:{token}"));
    }
}

#[test]
fn embedded_urls_identifiers_and_oversized_messages_are_not_payments() {
    for prefix in [
        "https://example.org/",
        "https://example.org/?token=",
        "abc",
        "thing:",
        "/",
    ] {
        for raw in [V3.trim(), INVOICE.trim()] {
            assert!(message_payment_links(format!("{prefix}{raw}")).is_empty());
        }
    }
    assert!(
        message_payment_links(format!("{} {}", "a".repeat(MAX_TEXT_BYTES), V3.trim())).is_empty()
    );
    assert_eq!(
        message_payment_links([V3.trim(); 8].join(" ")).len(),
        MAX_PAYMENTS
    );
    assert!(parse_cashu_token("a".repeat(MAX_TEXT_BYTES + 1)).is_none());
    assert!(parse_lightning_invoice("a".repeat(MAX_TEXT_BYTES + 1)).is_none());
}
