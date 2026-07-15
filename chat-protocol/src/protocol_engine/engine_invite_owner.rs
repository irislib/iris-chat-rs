const INVITE_OWNER_APP_KEYS_MAX_EVENT_BYTES: usize = 64 * 1024;
const INVITE_OWNER_APP_KEYS_MAX_DEVICES: usize = 64;
const INVITE_OWNER_APP_KEYS_MAX_FUTURE_SKEW_SECS: u64 = 5 * 60;

impl ProtocolEngine {
    fn update_invite_owner_app_keys_evidence(
        &mut self,
        owner: NdrOwnerPubkey,
        event: &Event,
        app_keys: &AppKeys,
    ) -> bool {
        if !invite_owner_app_keys_event_is_valid(event.pubkey, event, unix_now().get()) {
            return false;
        }
        let previous = self.invite_owner_app_keys_evidence.get(&owner).cloned();
        let created_at_secs = event.created_at.as_secs();
        let next = match previous.clone() {
            None => ProtocolAppKeysEvidence::Verified(Box::new(event.clone())),
            Some(ProtocolAppKeysEvidence::Ambiguous {
                created_at_secs: ambiguous_at,
            }) if created_at_secs > ambiguous_at => {
                ProtocolAppKeysEvidence::Verified(Box::new(event.clone()))
            }
            Some(ProtocolAppKeysEvidence::Ambiguous {
                created_at_secs: ambiguous_at,
            }) => ProtocolAppKeysEvidence::Ambiguous {
                created_at_secs: ambiguous_at,
            },
            Some(ProtocolAppKeysEvidence::Verified(current)) => {
                let current_created_at = current.created_at.as_secs();
                if created_at_secs > current_created_at {
                    ProtocolAppKeysEvidence::Verified(Box::new(event.clone()))
                } else if created_at_secs < current_created_at || current.id == event.id {
                    ProtocolAppKeysEvidence::Verified(current)
                } else {
                    let same_roster =
                        AppKeys::from_event(current.as_ref()).is_ok_and(|current_app_keys| {
                            app_keys_device_pubkeys(&current_app_keys)
                                == app_keys_device_pubkeys(app_keys)
                        });
                    if same_roster {
                        ProtocolAppKeysEvidence::Verified(if event.id < current.id {
                            Box::new(event.clone())
                        } else {
                            current
                        })
                    } else {
                        ProtocolAppKeysEvidence::Ambiguous { created_at_secs }
                    }
                }
            }
        };
        let changed = previous.as_ref() != Some(&next);
        self.invite_owner_app_keys_evidence.insert(owner, next);
        changed
    }

    pub fn import_private_invite_session_once(
        &mut self,
        response_event_id: &str,
        owner_pubkey: PublicKey,
        authenticated_device: PublicKey,
        state: SessionState,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolInviteSessionImportOutcome> {
        if self
            .processed_private_invite_response_ids
            .iter()
            .any(|processed| processed == response_event_id)
        {
            return Ok(ProtocolInviteSessionImportOutcome::AlreadyImported);
        }
        if owner_pubkey != authenticated_device {
            match self.invite_owner_app_keys_membership(
                ndr_owner(owner_pubkey),
                authenticated_device,
            ) {
                Some(true) => {}
                Some(false) => {
                    return Ok(ProtocolInviteSessionImportOutcome::Blocked(
                        ProtocolAcceptInviteBlock::UnauthorizedDevice {
                            owner_pubkey,
                            device_pubkey: authenticated_device,
                        },
                    ))
                }
                None => {
                    return Ok(ProtocolInviteSessionImportOutcome::Blocked(
                        ProtocolAcceptInviteBlock::MissingOwnerRoster {
                            owner_pubkey,
                            device_pubkey: authenticated_device,
                        },
                    ))
                }
            }
        }

        let checkpoint = self.state_checkpoint();
        self.session_manager.import_session_state(
            ndr_owner(owner_pubkey),
            ndr_device(authenticated_device),
            state,
            NdrUnixSeconds(now.get()),
        );
        self.processed_private_invite_response_ids
            .push(response_event_id.to_string());
        let excess = self
            .processed_private_invite_response_ids
            .len()
            .saturating_sub(PROCESSED_PRIVATE_INVITE_RESPONSE_LIMIT);
        if excess > 0 {
            self.processed_private_invite_response_ids.drain(0..excess);
        }
        self.invalidate_known_message_author_cache();
        if let Err(error) = self.persist() {
            self.restore_checkpoint(checkpoint);
            return Err(error);
        }
        let retry_batch = self.retry_pending_protocol(NdrUnixSeconds(now.get()))?;
        Ok(ProtocolInviteSessionImportOutcome::Imported(retry_batch))
    }

    fn invite_owner_app_keys_membership(
        &self,
        owner: NdrOwnerPubkey,
        device: PublicKey,
    ) -> Option<bool> {
        self.invite_owner_exact_app_keys_membership(owner, device)
    }

    fn invite_owner_exact_app_keys_membership(
        &self,
        owner: NdrOwnerPubkey,
        device: PublicKey,
    ) -> Option<bool> {
        match self.invite_owner_app_keys_evidence.get(&owner)? {
            ProtocolAppKeysEvidence::Verified(event) => AppKeys::from_event(event)
                .ok()
                .map(|app_keys| app_keys.get_device(&device).is_some()),
            ProtocolAppKeysEvidence::Ambiguous { .. } => None,
        }
    }

    /// Reads only existing protocol state and checks exact signed AppKeys
    /// evidence. It never creates protocol state or treats a generic
    /// cached roster projection as authorization.
    pub fn persisted_invite_owner_device_is_authorized(
        storage: Arc<dyn StorageAdapter>,
        local_owner_pubkey: PublicKey,
        local_device_keys: &Keys,
        invite_owner_pubkey: PublicKey,
        invite_device_pubkey: PublicKey,
    ) -> anyhow::Result<bool> {
        if invite_owner_pubkey == invite_device_pubkey {
            return Ok(true);
        }
        let local_owner = ndr_owner(local_owner_pubkey);
        let local_device = ndr_device(local_device_keys.public_key());
        let Some(engine) = Self::load_persisted_state(
            storage,
            local_owner_pubkey,
            local_owner,
            local_device,
            local_device_keys.secret_key().to_secret_bytes(),
        )?
        else {
            return Ok(false);
        };
        Ok(matches!(
            engine.invite_owner_exact_app_keys_membership(
                ndr_owner(invite_owner_pubkey),
                invite_device_pubkey,
            ),
            Some(true)
        ))
    }
}

fn sanitize_invite_owner_persisted_state(state: &mut ProtocolEnginePersistedState) {
    state.invite_owner_app_keys_evidence.retain(|owner, evidence| {
        match evidence {
            ProtocolAppKeysEvidence::Verified(event) => {
                public_owner(*owner).is_ok_and(|owner| {
                    invite_owner_app_keys_event_is_valid(owner, event, unix_now().get())
                })
            }
            ProtocolAppKeysEvidence::Ambiguous { .. } => true,
        }
    });
    let exact_verified_owners = state
        .invite_owner_app_keys_evidence
        .iter()
        .filter_map(|(owner, evidence)| {
            matches!(evidence, ProtocolAppKeysEvidence::Verified(_)).then_some(*owner)
        })
        .collect::<BTreeSet<_>>();
    if state.app_keys_provenance_version != PROTOCOL_APP_KEYS_PROVENANCE_VERSION {
        state.verified_app_keys_owners = exact_verified_owners.clone();
    } else {
        state
            .verified_app_keys_owners
            .extend(exact_verified_owners.iter().copied());
    }
    let excess = state
        .processed_private_invite_response_ids
        .len()
        .saturating_sub(PROCESSED_PRIVATE_INVITE_RESPONSE_LIMIT);
    if excess > 0 {
        state.processed_private_invite_response_ids.drain(0..excess);
    }
}

fn invite_owner_app_keys_event_is_valid(owner: PublicKey, event: &Event, now_secs: u64) -> bool {
    let device_tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("device"))
        .collect::<Vec<_>>();
    event.pubkey == owner
        && event.created_at.as_secs()
            <= now_secs.saturating_add(INVITE_OWNER_APP_KEYS_MAX_FUTURE_SKEW_SECS)
        && serde_json::to_vec(event)
            .is_ok_and(|bytes| bytes.len() <= INVITE_OWNER_APP_KEYS_MAX_EVENT_BYTES)
        && device_tags.len() <= INVITE_OWNER_APP_KEYS_MAX_DEVICES
        && device_tags.iter().all(|tag| {
            let values = tag.as_slice();
            values.len() >= 3 && values[2].parse::<u64>().is_ok()
        })
        && AppKeys::from_event(event).is_ok()
}
