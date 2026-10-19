use super::*;
use crate::user_profile::{SharedUserProfiles, UserProfileSnapshot};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct ProfileDirectory {
    pub shared: SharedUserProfiles,
    account: Option<String>,
    source_profiles: BTreeMap<String, OwnerProfileRecord>,
    discovery_revision: Option<u64>,
    candidates: BTreeMap<String, ProfileSearchCandidate>,
}

impl AppCore {
    pub(crate) fn set_shared_user_profiles(&mut self, shared: SharedUserProfiles) {
        self.profile_directory = ProfileDirectory {
            shared,
            ..Default::default()
        };
        self.refresh_user_profiles();
    }

    /// Publish once per changed input, before notifying shells. Lookups never
    /// enter the actor queue, acquire SQLite, or initiate network requests.
    pub(super) fn refresh_user_profiles(&mut self) {
        let account = self
            .logged_in
            .as_ref()
            .map(|session| session.owner_pubkey.to_hex());
        let directory = &self.profile_directory;
        if directory.account == account
            && directory.discovery_revision == Some(self.user_discovery_revision)
            && directory.source_profiles == self.owner_profiles
        {
            return;
        }
        if account.is_none() {
            self.profile_directory.candidates.clear();
        } else if directory.account != account
            || directory.discovery_revision != Some(self.user_discovery_revision)
        {
            // The bounded discovery index is read on discovery changes, never
            // by a rendered row. Canonical metadata always wins, including nil.
            match self.app_store.load_profile_directory_candidates() {
                Ok(candidates) => self.profile_directory.candidates = candidates,
                Err(error) => {
                    self.profile_directory.candidates.clear();
                    self.push_debug_log("profile.directory.read.error", error.to_string());
                }
            }
        }
        let mut next = BTreeMap::new();
        if let Some(local) = account.as_ref() {
            let owners: BTreeSet<_> = self
                .owner_profiles
                .keys()
                .chain(self.profile_directory.candidates.keys())
                .chain(self.user_discovery.users.keys())
                .chain(std::iter::once(local))
                .cloned()
                .collect();
            for owner in owners {
                let canonical = self.owner_profiles.get(&owner);
                let indexed = if canonical.is_none() {
                    self.profile_directory.candidates.get(&owner)
                } else {
                    None
                };
                let explicit = self
                    .owner_display_name(&owner)
                    .or_else(|| indexed.and_then(|p| profile_name_for_identity(&p.name, &owner)));
                let picture = canonical
                    .and_then(|p| p.picture.clone())
                    .or_else(|| indexed.and_then(|p| p.picture.clone()));
                let name = crate::present_person_name(
                    explicit.clone().unwrap_or_default(),
                    owner.clone(),
                    explicit,
                );
                next.insert(
                    owner.clone(),
                    UserProfileSnapshot {
                        owner_pubkey_hex: owner.clone(),
                        name,
                        picture_url: normalize_profile_url(picture),
                        social_connection: self.social_connection(&owner),
                    },
                );
            }
        }
        let previous = {
            let mut shared = self
                .profile_directory
                .shared
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if *shared == next {
                None
            } else {
                Some(std::mem::replace(&mut *shared, next))
            }
        };
        if previous.is_some() {
            self.bump_user_discovery_revision();
            self.state.user_discovery_revision = self.user_discovery_revision;
        }
        drop(previous); // Drop old metadata outside the UI reader's lock.
        self.profile_directory.account = account;
        self.profile_directory
            .source_profiles
            .clone_from(&self.owner_profiles);
        self.profile_directory.discovery_revision = Some(self.user_discovery_revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Kind, Timestamp};

    fn core() -> (tempfile::TempDir, AppCore) {
        let dir = tempfile::tempdir().unwrap();
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            dir.path().to_string_lossy().into_owned(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        let owner = Keys::generate();
        let device = Keys::generate();
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner),
            device_keys: device.clone(),
            client: Client::new(device),
            relay_urls: Vec::new(),
            authorization_state: LocalAuthorizationState::Authorized,
        });
        (dir, core)
    }

    fn profile(core: &AppCore, owner: &str) -> UserProfileSnapshot {
        core.profile_directory.shared.read().unwrap()[owner].clone()
    }

    #[test]
    fn shared_profile_precedence_removal_and_unchanged_emission() {
        let (_dir, mut core) = core();
        let person = Keys::generate();
        let owner = person.public_key().to_hex();
        let event = EventBuilder::new(
            Kind::Metadata,
            r#"{"display_name":"Ada","picture":"https://example.com/ada.jpg"}"#,
        )
        .custom_created_at(Timestamp::from_secs(10))
        .sign_with_keys(&person)
        .unwrap();
        assert!(core.apply_profile_metadata_event(&event));
        core.refresh_user_profiles();
        assert_eq!(profile(&core, &owner).name.name, "Ada");
        assert!(!profile(&core, &owner).name.is_fallback);
        let revision = core.user_discovery_revision;
        core.refresh_user_profiles();
        assert_eq!(core.user_discovery_revision, revision);
        core.owner_profiles
            .get_mut(&owner)
            .unwrap()
            .contact_memory
            .accepted_name = Some("Accepted".into());
        core.refresh_user_profiles();
        assert_eq!(profile(&core, &owner).name.name, "Accepted");
        core.owner_profiles.get_mut(&owner).unwrap().nickname = Some("Mum".into());
        core.refresh_user_profiles();
        assert_eq!(profile(&core, &owner).name.name, "Mum");
        let removed = EventBuilder::new(Kind::Metadata, r#"{"name":"Ada"}"#)
            .custom_created_at(Timestamp::from_secs(20))
            .sign_with_keys(&person)
            .unwrap();
        assert!(core.apply_profile_metadata_event(&removed));
        assert!(!core.apply_profile_metadata_event(&event));
        core.refresh_user_profiles();
        assert_eq!(profile(&core, &owner).name.name, "Mum");
        assert_eq!(profile(&core, &owner).picture_url, None);
    }

    #[test]
    fn discovery_hint_cannot_resurrect_canonical_photo_and_logout_clears_handle() {
        let (_dir, mut core) = core();
        let owner = Keys::generate().public_key().to_hex();
        core.app_store
            .upsert_profile_search_candidates(&[ProfileSearchCandidate {
                owner_pubkey_hex: owner.clone(),
                name: "Indexed".into(),
                aliases: vec![],
                nip05: None,
                picture: Some("https://example.com/old.jpg".into()),
                created_at_secs: 10,
            }])
            .unwrap();
        core.refresh_user_profiles();
        assert_eq!(profile(&core, &owner).name.name, "Indexed");
        let mut canonical = build_owner_profile_record("Current", None, None).unwrap();
        canonical.source_event_id = Some("signed-head".into());
        core.owner_profiles.insert(owner.clone(), canonical);
        core.refresh_user_profiles();
        assert_eq!(profile(&core, &owner).name.name, "Current");
        assert_eq!(profile(&core, &owner).picture_url, None);
        let shared = Arc::clone(&core.profile_directory.shared);
        core.logged_in = None;
        core.refresh_user_profiles();
        assert!(shared.read().unwrap().is_empty());
        core.set_shared_user_profiles(Arc::clone(&shared));
        assert!(shared.read().unwrap().is_empty());
    }
}
