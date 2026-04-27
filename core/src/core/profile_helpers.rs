use super::*;

impl OwnerProfileRecord {
    pub(super) fn preferred_label(&self) -> Option<String> {
        self.display_name.clone().or_else(|| self.name.clone())
    }
}

pub(super) fn normalize_profile_field(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(super) fn normalize_profile_url(value: Option<String>) -> Option<String> {
    let value = normalize_profile_field(value)?;
    (value.starts_with("https://") || value.starts_with("http://") || value.starts_with("htree://"))
        .then_some(value)
}

pub(super) fn build_owner_profile_record(
    name: &str,
    picture_url: Option<&str>,
) -> Option<OwnerProfileRecord> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(OwnerProfileRecord {
        name: Some(trimmed.to_string()),
        display_name: Some(trimmed.to_string()),
        picture: normalize_profile_url(picture_url.map(str::to_string)),
        updated_at_secs: unix_now().get(),
    })
}

pub(super) fn parse_owner_profile_record(
    content: &str,
    updated_at_secs: u64,
) -> Option<OwnerProfileRecord> {
    let parsed = serde_json::from_str::<NostrProfileMetadata>(content).ok()?;
    let name = normalize_profile_field(parsed.name);
    let display_name = normalize_profile_field(parsed.display_name);
    let picture = normalize_profile_url(parsed.picture);
    if name.is_none() && display_name.is_none() && picture.is_none() {
        return None;
    }

    Some(OwnerProfileRecord {
        name,
        display_name,
        picture,
        updated_at_secs,
    })
}

pub(super) fn build_profile_metadata_json(profile: &OwnerProfileRecord) -> String {
    let name = profile
        .name
        .clone()
        .or_else(|| profile.display_name.clone())
        .unwrap_or_default();
    let display_name = profile.display_name.clone().or_else(|| Some(name.clone()));
    serde_json::to_string(&NostrProfileMetadata {
        name: (!name.is_empty()).then_some(name.clone()),
        display_name,
        picture: profile.picture.clone(),
    })
    .unwrap_or_else(|_| format!(r#"{{"name":"{name}","display_name":"{name}"}}"#))
}
