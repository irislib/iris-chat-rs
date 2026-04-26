use super::*;

impl AppCore {
    pub(super) fn set_local_profile_name(&mut self, name: &str) {
        let picture_url = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| {
                self.owner_profiles
                    .get(&logged_in.owner_pubkey.to_string())
                    .and_then(|profile| profile.picture.as_deref())
            })
            .map(str::to_string);
        self.set_local_profile_metadata(name, picture_url.as_deref());
    }

    pub(super) fn set_local_profile_metadata(&mut self, name: &str, picture_url: Option<&str>) {
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_string())
        else {
            return;
        };

        let Some(record) = build_owner_profile_record(name, picture_url) else {
            return;
        };

        self.owner_profiles.insert(local_owner_hex.clone(), record);
        self.push_debug_log("profile.local.set", format!("owner={local_owner_hex}"));
        self.persist_best_effort();
    }

    pub(super) fn update_profile_metadata(&mut self, name: &str, picture_url: Option<&str>) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.state.toast = Some("Display name is required.".to_string());
            self.emit_state();
            return;
        }
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        };
        if logged_in.owner_keys.is_none() {
            self.state.toast = Some("Owner key is required to edit profile.".to_string());
            self.emit_state();
            return;
        }
        let normalized_picture_url = match normalize_profile_field(picture_url.map(str::to_string))
        {
            Some(url) if normalize_profile_url(Some(url.clone())).is_none() => {
                self.state.toast =
                    Some("Profile picture must be an http or https URL.".to_string());
                self.emit_state();
                return;
            }
            value => value,
        };

        self.set_local_profile_metadata(trimmed, normalized_picture_url.as_deref());
        self.republish_local_identity_artifacts();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn upload_profile_picture(&mut self, file_path: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore an account first.".to_string());
            self.emit_state();
            return;
        };
        let Some(owner_keys) = logged_in.owner_keys.as_ref() else {
            self.state.toast = Some("Owner key is required to edit profile.".to_string());
            self.emit_state();
            return;
        };
        let path = PathBuf::from(file_path.trim());
        if !path.is_file() {
            self.state.toast = Some("Profile picture was not found.".to_string());
            self.emit_state();
            return;
        }
        let secret_hex = owner_keys.secret_key().to_secret_hex();
        let sender = self.core_sender.clone();
        self.state.busy.uploading_attachment = true;
        self.emit_state();
        self.runtime.spawn(async move {
            let result = upload_profile_picture_to_blossom(&secret_hex, &path)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::ProfilePictureUploadFinished { result },
            )));
        });
    }

    pub(super) fn handle_profile_picture_upload_finished(
        &mut self,
        result: Result<String, String>,
    ) {
        self.state.busy.uploading_attachment = false;
        match result {
            Ok(picture_url) => {
                self.push_debug_log(
                    "profile.picture.upload.ok",
                    format!("url={picture_url}"),
                );
                let name = self
                    .state
                    .account
                    .as_ref()
                    .map(|account| account.display_name.clone())
                    .unwrap_or_else(|| "Iris".to_string());
                self.update_profile_metadata(&name, Some(&picture_url));
                self.state.toast = Some("Profile picture updated.".to_string());
                self.emit_state();
            }
            Err(error) => {
                self.push_debug_log("profile.picture.upload.error", error.clone());
                self.state.toast = Some(format!("Profile picture upload failed: {error}"));
                self.emit_state();
            }
        }
    }

    pub(super) fn apply_profile_metadata_event(&mut self, event: &Event) -> bool {
        let owner_hex = event.pubkey.to_hex();
        let Some(record) = parse_owner_profile_record(&event.content, event.created_at.as_u64())
        else {
            return false;
        };

        if let Some(existing) = self.owner_profiles.get(&owner_hex) {
            if existing.updated_at_secs > record.updated_at_secs {
                return false;
            }
        }

        self.owner_profiles.insert(owner_hex.clone(), record);
        self.push_debug_log("relay.metadata", format!("owner={owner_hex}"));
        true
    }

    pub(super) fn owner_display_name(&self, owner_hex: &str) -> Option<String> {
        self.owner_profiles
            .get(owner_hex)
            .and_then(OwnerProfileRecord::preferred_label)
    }

    pub(super) fn owner_display_label(&self, owner_hex: &str) -> String {
        self.owner_display_name(owner_hex)
            .unwrap_or_else(|| fallback_profile_name_for_identity(owner_hex))
    }

    pub(super) fn owner_secondary_identifier(&self, owner_hex: &str) -> Option<String> {
        let _ = owner_hex;
        None
    }
}

fn fallback_profile_name_for_identity(identity: &str) -> String {
    const ADJECTIVES: [&str; 12] = [
        "Amber", "Bright", "Calm", "Clear", "Golden", "Lunar", "Nova", "Quiet", "Silver", "Solar",
        "Velvet", "Wild",
    ];
    const NOUNS: [&str; 12] = [
        "Aurora", "Comet", "Echo", "Falcon", "Harbor", "Listener", "Otter", "Raven", "Signal",
        "Sparrow", "Tide", "Voyager",
    ];

    let trimmed = identity.trim();
    if trimmed.is_empty() {
        return "Quiet Listener".to_string();
    }

    let hash = trimmed.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let adjective = ADJECTIVES[(hash as usize) % ADJECTIVES.len()];
    let noun = NOUNS[((hash as usize) / ADJECTIVES.len()) % NOUNS.len()];
    format!("{adjective} {noun}")
}
