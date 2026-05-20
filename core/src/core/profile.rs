use super::*;

const PROFILE_METADATA_FETCH_TIMEOUT_SECS: u64 = 5;

impl AppCore {
    pub(super) fn fetch_missing_profile_metadata(
        &mut self,
        owner_input: &str,
        reason: &'static str,
    ) -> bool {
        let Ok((owner_hex, owner_pubkey)) = parse_peer_input(owner_input) else {
            return false;
        };
        self.fetch_missing_profile_metadata_for_owner(owner_hex, owner_pubkey, reason)
    }

    fn fetch_missing_profile_metadata_for_owner(
        &mut self,
        owner_hex: String,
        owner_pubkey: PublicKey,
        reason: &'static str,
    ) -> bool {
        if is_group_chat_id(&owner_hex) || self.has_cached_profile_metadata(&owner_hex) {
            return false;
        }
        let Some((client, relay_urls)) = self
            .logged_in
            .as_ref()
            .filter(|logged_in| !logged_in.relay_urls.is_empty())
            .map(|logged_in| (logged_in.client.clone(), logged_in.relay_urls.clone()))
        else {
            return false;
        };
        if !self
            .profile_metadata_fetch_inflight
            .insert(owner_hex.clone())
        {
            self.push_debug_log(
                "profile.metadata.fetch.skip",
                format!("reason={reason} owner={owner_hex} in_flight=true"),
            );
            return false;
        }

        self.push_debug_log(
            "profile.metadata.fetch",
            format!("reason={reason} owner={owner_hex}"),
        );
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(5)).await;
            let filter = Filter::new().kind(Kind::Metadata).author(owner_pubkey);
            match client
                .fetch_events(
                    filter,
                    Duration::from_secs(PROFILE_METADATA_FETCH_TIMEOUT_SECS),
                )
                .await
            {
                Ok(events) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::ProfileMetadataFetchFinished {
                            owner_pubkey_hex: owner_hex,
                            events: events.iter().cloned().collect(),
                            error: None,
                        },
                    )));
                }
                Err(error) => {
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::ProfileMetadataFetchFinished {
                            owner_pubkey_hex: owner_hex,
                            events: Vec::new(),
                            error: Some(error.to_string()),
                        },
                    )));
                }
            }
        });
        true
    }

    fn has_cached_profile_metadata(&self, owner_hex: &str) -> bool {
        self.owner_profiles.get(owner_hex).is_some_and(|profile| {
            profile.profile_label().is_some()
                || profile.picture.is_some()
                || profile.about.is_some()
                || profile.extra_metadata_json.trim() != "{}"
                || !profile.extra_tags.is_empty()
        })
    }

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
        let about = self
            .logged_in
            .as_ref()
            .and_then(|logged_in| {
                self.owner_profiles
                    .get(&logged_in.owner_pubkey.to_string())
                    .and_then(|profile| profile.about.as_deref())
            })
            .map(str::to_string);
        self.set_local_profile_metadata(name, picture_url.as_deref(), about.as_deref());
    }

    pub(super) fn set_local_profile_metadata(
        &mut self,
        name: &str,
        picture_url: Option<&str>,
        about: Option<&str>,
    ) {
        let Some(local_owner_hex) = self
            .logged_in
            .as_ref()
            .map(|logged_in| logged_in.owner_pubkey.to_string())
        else {
            return;
        };

        let Some(record) = build_owner_profile_record(name, picture_url, about) else {
            return;
        };

        let mut record = record;
        if let Some(existing) = self.owner_profiles.get(&local_owner_hex) {
            record.nickname = existing.nickname.clone();
            record.extra_metadata_json = existing.extra_metadata_json.clone();
            record.extra_tags = existing.extra_tags.clone();
        }
        self.owner_profiles.insert(local_owner_hex.clone(), record);
        self.push_debug_log("profile.local.set", format!("owner={local_owner_hex}"));
        self.persist_best_effort();
    }

    pub(super) fn set_contact_nickname(&mut self, owner_pubkey_hex: &str, nickname: &str) {
        let trimmed_owner = owner_pubkey_hex.trim();
        if trimmed_owner.is_empty() || is_group_chat_id(trimmed_owner) {
            self.state.toast = Some("User ID is invalid.".to_string());
            self.emit_state();
            return;
        }
        let Ok(owner) = PublicKey::parse(trimmed_owner) else {
            self.state.toast = Some("User ID is invalid.".to_string());
            self.emit_state();
            return;
        };
        let owner_hex = owner.to_hex();
        if self
            .logged_in
            .as_ref()
            .is_some_and(|logged_in| logged_in.owner_pubkey == owner)
        {
            self.state.toast = Some("Use your profile name for yourself.".to_string());
            self.emit_state();
            return;
        }
        if !self.threads.contains_key(&owner_hex) {
            self.state.toast = Some("Chat was not found.".to_string());
            self.emit_state();
            return;
        }

        let next_nickname = normalize_nickname_field(nickname);
        if next_nickname
            .as_ref()
            .is_some_and(|value| value.chars().count() > MAX_CONTACT_NICKNAME_CHARS)
        {
            self.state.toast = Some("Nickname is too long.".to_string());
            self.emit_state();
            return;
        }

        let previous_nickname = self
            .owner_profiles
            .get(&owner_hex)
            .and_then(|profile| profile.nickname.clone());
        if previous_nickname == next_nickname {
            self.state.toast = Some(if next_nickname.is_some() {
                "Nickname saved".to_string()
            } else {
                "Nickname removed".to_string()
            });
            self.emit_state();
            return;
        }

        if let Some(existing) = self.owner_profiles.get_mut(&owner_hex) {
            existing.nickname = next_nickname.clone();
            if existing.is_empty() {
                self.owner_profiles.remove(&owner_hex);
            }
        } else if let Some(nickname) = next_nickname.clone() {
            self.owner_profiles.insert(
                owner_hex.clone(),
                OwnerProfileRecord {
                    nickname: Some(nickname),
                    ..OwnerProfileRecord::default()
                },
            );
        }

        self.push_debug_log("profile.nickname.set", format!("owner={owner_hex}"));
        self.state.toast = Some(if next_nickname.is_some() {
            "Nickname saved".to_string()
        } else {
            "Nickname removed".to_string()
        });
        self.persist_best_effort();
        // Mobile-push snapshot embeds peer display labels.
        self.mark_mobile_push_dirty();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn update_profile_metadata(
        &mut self,
        name: &str,
        picture_url: Option<&str>,
        about: Option<&str>,
    ) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.state.toast = Some("Display name is required.".to_string());
            self.emit_state();
            return;
        }
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore a profile first.".to_string());
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

        self.set_local_profile_metadata(trimmed, normalized_picture_url.as_deref(), about);
        self.republish_local_identity_artifacts();
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn delete_profile_metadata(&mut self) {
        let Some((owner_hex, owner_keys)) = self.logged_in.as_ref().and_then(|logged_in| {
            logged_in
                .owner_keys
                .clone()
                .map(|keys| (logged_in.owner_pubkey.to_hex(), keys))
        }) else {
            self.state.toast = Some("Secret key is required to delete profile.".to_string());
            self.emit_state();
            return;
        };

        self.owner_profiles.remove(&owner_hex);
        self.persist_best_effort();

        match EventBuilder::new(Kind::Metadata, "{}").sign_with_keys(&owner_keys) {
            Ok(event) => {
                if self.publish_runtime_event(event, "profile-delete", None) {
                    self.state.toast = Some("Profile deleted".to_string());
                } else {
                    self.state.toast = Some("Profile could not be deleted.".to_string());
                }
            }
            Err(error) => {
                self.push_debug_log("profile.delete.error", error.to_string());
                self.state.toast = Some("Profile could not be deleted.".to_string());
            }
        }

        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn upload_profile_picture(&mut self, file_path: &str) {
        self.push_debug_log("profile.picture.upload.start", format!("path={file_path}"));
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.push_debug_log("profile.picture.upload.skip", "no_logged_in".to_string());
            self.state.toast = Some("Create or restore a profile first.".to_string());
            self.emit_state();
            return;
        };
        let Some(owner_keys) = logged_in.owner_keys.as_ref() else {
            self.push_debug_log("profile.picture.upload.skip", "no_owner_keys".to_string());
            self.state.toast = Some("Owner key is required to edit profile.".to_string());
            self.emit_state();
            return;
        };
        let path = PathBuf::from(file_path.trim());
        if !path.is_file() {
            self.push_debug_log(
                "profile.picture.upload.skip",
                format!("missing_file={file_path}"),
            );
            self.state.toast = Some("Profile picture was not found.".to_string());
            self.emit_state();
            return;
        }
        let secret_hex = owner_keys.secret_key().to_secret_hex();
        let sender = self.core_sender.clone();
        self.state.busy.uploading_attachment = true;
        self.emit_state();
        self.runtime.spawn(async move {
            let result = upload_profile_picture_to_hashtree(&secret_hex, &path)
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
                self.push_debug_log("profile.picture.upload.ok", format!("url={picture_url}"));
                let name = self
                    .state
                    .account
                    .as_ref()
                    .map(|account| account.display_name.clone())
                    .unwrap_or_else(|| "Iris".to_string());
                let about = self
                    .state
                    .account
                    .as_ref()
                    .and_then(|account| account.about.clone());
                self.update_profile_metadata(&name, Some(&picture_url), about.as_deref());
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
        let extra_tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        let Some(mut record) =
            parse_owner_profile_record(&event.content, extra_tags, event.created_at.as_secs())
        else {
            return false;
        };

        if let Some(existing) = self.owner_profiles.get(&owner_hex) {
            if existing.updated_at_secs > record.updated_at_secs {
                return false;
            }
            record.nickname = existing.nickname.clone();
        }

        self.owner_profiles.insert(owner_hex.clone(), record);
        self.push_debug_log("relay.metadata", format!("owner={owner_hex}"));
        // Mobile-push snapshot embeds the display label per session.
        self.mark_mobile_push_dirty();
        true
    }

    pub(super) fn owner_display_name(&self, owner_hex: &str) -> Option<String> {
        self.owner_profiles
            .get(owner_hex)
            .and_then(OwnerProfileRecord::preferred_label)
    }

    pub(super) fn owner_nickname(&self, owner_hex: &str) -> Option<String> {
        self.owner_profiles
            .get(owner_hex)
            .and_then(|profile| profile.nickname.clone())
    }

    pub(super) fn owner_profile_name(&self, owner_hex: &str) -> Option<String> {
        self.owner_profiles
            .get(owner_hex)
            .and_then(OwnerProfileRecord::profile_label)
    }

    pub(super) fn owner_display_label(&self, owner_hex: &str) -> String {
        self.owner_display_name(owner_hex)
            .unwrap_or_else(|| fallback_profile_name_for_identity(owner_hex))
    }

    pub(super) fn owner_picture_url(&self, owner_hex: &str) -> Option<String> {
        self.owner_profiles
            .get(owner_hex)
            .and_then(|profile| profile.picture.clone())
    }

    pub(super) fn owner_about(&self, owner_hex: &str) -> Option<String> {
        self.owner_profiles
            .get(owner_hex)
            .and_then(|profile| profile.about.clone())
    }

    pub(super) fn owner_secondary_identifier(&self, owner_hex: &str) -> Option<String> {
        let nickname = self.owner_nickname(owner_hex)?;
        let profile_name = self.owner_profile_name(owner_hex)?;
        (profile_name != nickname).then_some(profile_name)
    }
}

const MAX_CONTACT_NICKNAME_CHARS: usize = 80;

fn normalize_nickname_field(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    normalize_profile_field(Some(normalized))
}

pub(super) fn fallback_profile_name_for_identity(identity: &str) -> String {
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
    let adjective = ADJECTIVES
        .get((hash as usize) % ADJECTIVES.len())
        .copied()
        .unwrap_or("Quiet");
    let noun = NOUNS
        .get(((hash as usize) / ADJECTIVES.len()) % NOUNS.len())
        .copied()
        .unwrap_or("Listener");
    format!("{adjective} {noun}")
}
