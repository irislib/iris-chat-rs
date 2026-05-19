use super::*;

impl OwnerProfileRecord {
    pub(super) fn preferred_label(&self) -> Option<String> {
        self.nickname.clone().or_else(|| self.profile_label())
    }

    pub(super) fn profile_label(&self) -> Option<String> {
        self.display_name.clone().or_else(|| self.name.clone())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.nickname.is_none()
            && self.name.is_none()
            && self.display_name.is_none()
            && self.picture.is_none()
            && self.about.is_none()
            && self.extra_tags.is_empty()
            && parsed_extra_metadata_object(&self.extra_metadata_json).is_empty()
    }
}

fn parsed_extra_metadata_object(raw: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
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
    about: Option<&str>,
) -> Option<OwnerProfileRecord> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(OwnerProfileRecord {
        nickname: None,
        name: Some(trimmed.to_string()),
        display_name: Some(trimmed.to_string()),
        picture: normalize_profile_url(picture_url.map(str::to_string)),
        about: normalize_profile_field(about.map(str::to_string)),
        extra_metadata_json: default_extra_metadata_json_str(),
        extra_tags: Vec::new(),
        updated_at_secs: unix_now().get(),
    })
}

pub(super) fn parse_owner_profile_record(
    content: &str,
    extra_tags: Vec<Vec<String>>,
    updated_at_secs: u64,
) -> Option<OwnerProfileRecord> {
    let parsed = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let mut object = match parsed {
        serde_json::Value::Object(map) => map,
        _ => return None,
    };

    let name = normalize_profile_field(remove_string_field(&mut object, "name"));
    let display_name = normalize_profile_field(remove_string_field(&mut object, "display_name"));
    let picture = normalize_profile_url(remove_string_field(&mut object, "picture"));
    let about = normalize_profile_field(remove_string_field(&mut object, "about"));

    if name.is_none()
        && display_name.is_none()
        && picture.is_none()
        && about.is_none()
        && object.is_empty()
        && extra_tags.is_empty()
    {
        return None;
    }

    let extra_metadata_json = serde_json::to_string(&serde_json::Value::Object(object))
        .unwrap_or_else(|_| default_extra_metadata_json_str());

    Some(OwnerProfileRecord {
        nickname: None,
        name,
        display_name,
        picture,
        about,
        extra_metadata_json,
        extra_tags,
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

    let mut object = parsed_extra_metadata_object(&profile.extra_metadata_json);
    set_or_remove_string(&mut object, "name", (!name.is_empty()).then(|| name.clone()));
    set_or_remove_string(&mut object, "display_name", display_name);
    set_or_remove_string(&mut object, "picture", profile.picture.clone());
    set_or_remove_string(&mut object, "about", profile.about.clone());

    serde_json::to_string(&serde_json::Value::Object(object))
        .unwrap_or_else(|_| format!(r#"{{"name":"{name}","display_name":"{name}"}}"#))
}

fn default_extra_metadata_json_str() -> String {
    "{}".to_string()
}

fn remove_string_field(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    match object.remove(key)? {
        serde_json::Value::String(value) => Some(value),
        // Non-string values are unexpected for these fields; drop them.
        _ => None,
    }
}

fn set_or_remove_string(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<String>,
) {
    match value {
        Some(value) => {
            object.insert(key.to_string(), serde_json::Value::String(value));
        }
        None => {
            object.remove(key);
        }
    }
}
