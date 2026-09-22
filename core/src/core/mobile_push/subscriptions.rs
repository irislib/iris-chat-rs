use super::*;

pub(crate) fn resolve_mobile_push_server_url(
    platform_key: String,
    is_release: bool,
    override_url: Option<String>,
) -> String {
    let trimmed_override = override_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(value) = trimmed_override {
        return value.to_string();
    }
    let platform = platform_key.trim().to_ascii_lowercase();
    if !is_release && matches!(platform.as_str(), "ios" | "android") {
        return MOBILE_PUSH_SANDBOX_SERVER_URL.to_string();
    }
    MOBILE_PUSH_PRODUCTION_SERVER_URL.to_string()
}

pub(crate) fn mobile_push_stored_subscription_id_key(platform_key: String) -> String {
    format!(
        "settings.mobile_push_subscription_id.{}",
        normalize_platform_key(&platform_key)
    )
}

pub(crate) fn build_mobile_push_list_subscriptions_request(
    owner_nsec: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    build_mobile_push_subscription_request(
        owner_nsec,
        "GET",
        "/subscriptions",
        None,
        platform_key,
        is_release,
        server_url_override,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_mobile_push_create_subscription_request(
    owner_nsec: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    background_message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let body_json = mobile_push_subscription_body_json(
        &platform_key,
        &push_token,
        apns_topic.as_deref(),
        message_author_pubkeys,
        background_message_author_pubkeys,
        invite_response_pubkeys,
        is_release,
    )?;
    build_mobile_push_subscription_request(
        owner_nsec,
        "POST",
        "/subscriptions",
        Some(body_json),
        platform_key,
        is_release,
        server_url_override,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_mobile_push_update_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    background_message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let subscription_id = normalize_path_component(&subscription_id)?;
    let body_json = mobile_push_subscription_body_json(
        &platform_key,
        &push_token,
        apns_topic.as_deref(),
        message_author_pubkeys,
        background_message_author_pubkeys,
        invite_response_pubkeys,
        is_release,
    )?;
    build_mobile_push_subscription_request(
        owner_nsec,
        "POST",
        &format!("/subscriptions/{subscription_id}"),
        Some(body_json),
        platform_key,
        is_release,
        server_url_override,
    )
}

pub(crate) fn build_mobile_push_delete_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let subscription_id = normalize_path_component(&subscription_id)?;
    build_mobile_push_subscription_request(
        owner_nsec,
        "DELETE",
        &format!("/subscriptions/{subscription_id}"),
        None,
        platform_key,
        is_release,
        server_url_override,
    )
}

fn build_mobile_push_subscription_request(
    owner_nsec: String,
    method: &str,
    path: &str,
    body_json: Option<String>,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let method = method.trim().to_ascii_uppercase();
    let base_url = resolve_mobile_push_server_url(platform_key, is_release, server_url_override);
    let url = resolve_mobile_push_url(&base_url, path)?;
    let authorization_header = build_mobile_push_auth_header(&owner_nsec, &method, &url)?;
    Some(MobilePushSubscriptionRequest {
        method,
        url,
        authorization_header,
        body_json,
    })
}

fn build_mobile_push_auth_header(owner_nsec: &str, method: &str, url: &str) -> Option<String> {
    let keys = Keys::parse(owner_nsec.trim()).ok()?;
    let event = EventBuilder::new(Kind::from(MOBILE_PUSH_AUTH_KIND), "")
        .tag(Tag::parse(["u", url]).ok()?)
        .tag(Tag::parse(["method", method]).ok()?)
        .sign_with_keys(&keys)
        .ok()?;
    let encoded = BASE64_STANDARD.encode(serde_json::to_vec(&event).ok()?);
    Some(format!("Nostr {encoded}"))
}

fn mobile_push_subscription_body_json(
    platform_key: &str,
    push_token: &str,
    apns_topic: Option<&str>,
    message_author_pubkeys: Vec<String>,
    background_message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
) -> Option<String> {
    let platform = normalize_platform_key(platform_key);
    let token = push_token.trim();
    if token.is_empty() {
        return None;
    }
    let authors = normalize_hex_list(message_author_pubkeys);
    let background_authors = normalize_hex_list(background_message_author_pubkeys)
        .into_iter()
        .filter(|author| authors.contains(author))
        .collect::<Vec<_>>();
    let invite_response_pubkeys = normalize_hex_list(invite_response_pubkeys);
    if authors.is_empty() && invite_response_pubkeys.is_empty() {
        return None;
    }
    let mut filters = Vec::new();
    if !authors.is_empty() {
        filters.push(serde_json::json!({
            "kinds": [MOBILE_PUSH_OUTER_MESSAGE_EVENT_KIND],
            "authors": authors,
        }));
    }
    if !invite_response_pubkeys.is_empty() {
        filters.push(serde_json::json!({
            "kinds": [MOBILE_PUSH_INVITE_RESPONSE_KIND],
            "#p": invite_response_pubkeys,
        }));
    }
    let primary_filter = filters.first()?.clone();

    let mut payload = serde_json::json!({
        "webhooks": [],
        "web_push_subscriptions": [],
        "fcm_tokens": if platform == "android" { vec![token.to_string()] } else { Vec::<String>::new() },
        "apns_tokens": if platform == "ios" { vec![token.to_string()] } else { Vec::<String>::new() },
        "filter": primary_filter,
        "filters": filters,
        "background_authors": background_authors,
    });
    if platform == "ios" {
        if let Some(topic) = apns_topic.map(str::trim).filter(|value| !value.is_empty()) {
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "apns_topic".to_string(),
                    serde_json::Value::String(topic.to_string()),
                );
                object.insert(
                    "apns_environment".to_string(),
                    serde_json::Value::String(
                        if is_release {
                            "production"
                        } else {
                            "development"
                        }
                        .to_string(),
                    ),
                );
            }
        }
    }
    serde_json::to_string(&payload).ok()
}

fn resolve_mobile_push_url(base_url: &str, path: &str) -> Option<String> {
    let mut url = url::Url::parse(base_url.trim()).ok()?;
    let base_path = url.path().trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    url.set_path(&format!("{base_path}/{normalized_path}"));
    Some(url.to_string())
}

fn normalize_platform_key(platform_key: &str) -> String {
    match platform_key.trim().to_ascii_lowercase().as_str() {
        "ios" => "ios".to_string(),
        "android" => "android".to_string(),
        _ => "unsupported".to_string(),
    }
}

fn normalize_path_component(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_hex_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = HashSet::new();
    for value in values {
        let candidate = value.trim().to_ascii_lowercase();
        if candidate.len() == 64 && candidate.chars().all(|char| char.is_ascii_hexdigit()) {
            normalized.insert(candidate);
        }
    }
    sorted_hexes(normalized)
}
