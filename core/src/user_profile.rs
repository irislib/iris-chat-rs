use crate::{present_person_name, PersonNamePresentation, SocialConnectionSnapshot};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Current account's presentation of a person. Metadata stays in the core;
/// native views select an identity, rather than passing cached names/photos.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct UserProfileSnapshot {
    pub owner_pubkey_hex: String,
    pub name: PersonNamePresentation,
    pub picture_url: Option<String>,
    pub social_connection: Option<SocialConnectionSnapshot>,
}

impl UserProfileSnapshot {
    pub(crate) fn unknown(owner: String) -> Self {
        Self {
            name: present_person_name(String::new(), owner.clone(), None),
            owner_pubkey_hex: owner,
            picture_url: None,
            social_connection: None,
        }
    }
}

pub(crate) type SharedUserProfiles = Arc<RwLock<BTreeMap<String, UserProfileSnapshot>>>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ffi_lookup_is_direct_and_normalizes_identity_without_an_actor() {
        use nostr::ToBech32;
        let app = crate::ffi_app_failure("No worker".into());
        let owner = "ab".repeat(32);
        let mut known = UserProfileSnapshot::unknown(owner.clone());
        known.name = PersonNamePresentation {
            name: "Ada".into(),
            is_fallback: false,
        };
        app.shared_user_profiles
            .write()
            .unwrap()
            .insert(owner.clone(), known.clone());
        let encoded = nostr::PublicKey::from_hex(&owner)
            .unwrap()
            .to_bech32()
            .unwrap();
        assert_eq!(app.user_profile(encoded), known);
        assert!(app.user_profile("cd".repeat(32)).name.is_fallback);
        app.shared_user_profiles.write().unwrap().clear();
        let after = app.user_profile(owner);
        assert!(after.name.is_fallback);
        assert!(after.picture_url.is_none());
    }
}
