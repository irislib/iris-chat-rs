use super::account_app_keys::normalize_device_label;
use super::*;

impl AppCore {
    pub(super) fn add_authorized_device(&mut self, device_input: &str) {
        let Some(logged_in) = self.logged_in.as_ref() else {
            self.state.toast = Some("Create or restore a profile first.".to_string());
            self.emit_state();
            return;
        };
        if logged_in.owner_keys.is_none() {
            self.state.toast = Some("Only the primary device can manage devices.".to_string());
            self.emit_state();
            return;
        }

        let bootstrap =
            match parse_nostr_identity_device_approval_bootstrap(device_input.trim(), &[]) {
                Ok(Some(bootstrap)) => bootstrap,
                _ => {
                    self.state.toast = Some("Invalid device request.".to_string());
                    self.emit_state();
                    return;
                }
            };
        if self.device_approval_relay_urls.len() != 1 {
            self.state.toast = Some("Could not approve device request.".to_string());
            self.emit_state();
            return;
        }
        self.state.busy.updating_roster = true;
        self.state.toast = None;
        self.emit_state();

        let result = self.accept_link_device_approval_bootstrap(bootstrap);
        self.state.busy.updating_roster = false;
        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        } else {
            self.state.toast = Some("Device added".to_string());
        }
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    fn accept_link_device_approval_bootstrap(
        &mut self,
        bootstrap: NostrIdentityDeviceApprovalBootstrap,
    ) -> anyhow::Result<()> {
        let approval_relay_urls = self.device_approval_relay_urls.clone();
        if approval_relay_urls.len() != 1 {
            anyhow::bail!("Device approval requires exactly one approval relay.");
        }
        let logged_in = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
        if logged_in.owner_keys.is_none() {
            anyhow::bail!("Only the primary device can manage devices.");
        }
        let approver_keys = logged_in.device_keys.clone();
        let owner_pubkey = logged_in.owner_pubkey;
        let owner_hex = owner_pubkey.to_hex();
        let profile_id = account::nostr_identity_profile_id_for_owner(owner_pubkey);
        let approval_content = approve_nostr_identity_device_approval_bootstrap(
            ApproveNostrIdentityDeviceApprovalBootstrapOptions {
                bootstrap: bootstrap.clone(),
                profile_id,
                roster_ops: Vec::new(),
                approved_by_pubkey: approver_keys.public_key().to_hex(),
                approved_at: i64::try_from(unix_now().get()).unwrap_or(i64::MAX),
                client_nonce: None,
                capabilities: None,
            },
        )?;
        let NostrIdentityRosterOp::AddFacet { facet } = &approval_content.op else {
            anyhow::bail!("Shared device approval did not add a device.");
        };
        let device_app_key_pubkey = PublicKey::from_hex(&facet.pubkey)
            .map_err(|error| anyhow::anyhow!("Invalid device key: {error}"))?;
        let invite = account::device_approval_pairing_invite(
            device_app_key_pubkey,
            &bootstrap.request_secret,
        )?;
        let request_labels = CurrentDeviceLabels {
            device_label: bootstrap.label.as_deref().and_then(normalize_device_label),
            client_label: None,
        };
        let request_labels =
            if request_labels.device_label.is_some() || request_labels.client_label.is_some() {
                Some(request_labels)
            } else {
                None
            };
        let previous_app_keys = self.app_keys.get(&owner_hex).cloned();
        self.upsert_local_app_key_device_with_labels(
            owner_pubkey,
            device_app_key_pubkey,
            request_labels.as_ref(),
            true,
        );
        let result = (|| {
            let signed_roster_event = build_nostr_identity_roster_op_event_with_client_nonce(
                &approver_keys,
                approval_content.profile_id,
                approval_content.parents.clone(),
                approval_content.actor_seq,
                approval_content.op.clone(),
                approval_content.created_at,
                approval_content.client_nonce.clone(),
                None,
            )?;
            let request_pubkey = PublicKey::parse(&bootstrap.request_npub)
                .map_err(|error| anyhow::anyhow!("Invalid approval request key: {error}"))?;
            let receipt = NostrIdentityDeviceApprovalReceipt {
                schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
                profile_id: approval_content.profile_id,
                request_pubkey: request_pubkey.to_hex(),
                device_app_key_pubkey: facet.pubkey.clone(),
                approved_by_pubkey: approval_content.actor_pubkey.clone(),
                approved_at: approval_content.created_at,
                request_secret: bootstrap.request_secret.clone(),
                subject_pubkey: Some(owner_hex.clone()),
                roster_op_id: Some(signed_roster_event.id.to_string()),
                signed_roster_event: Some(serde_json::to_string(&signed_roster_event)?),
            };
            let receipt_event =
                build_nostr_identity_device_approval_receipt_event(&approver_keys, receipt)?;
            self.accept_link_device_invite_session(invite, approval_relay_urls, receipt_event)
        })();
        if result.is_err() {
            if let Some(previous_app_keys) = previous_app_keys {
                self.app_keys.insert(owner_hex, previous_app_keys);
            } else {
                self.app_keys.remove(&owner_hex);
            }
        }
        result
    }

    fn accept_link_device_invite_session(
        &mut self,
        invite: Invite,
        approval_relay_urls: Vec<RelayUrl>,
        receipt_event: Event,
    ) -> anyhow::Result<()> {
        let logged_in = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
        if logged_in.owner_keys.is_none() {
            return Err(anyhow::anyhow!(
                "Only the primary device can manage devices."
            ));
        }
        let owner_pubkey = logged_in.owner_pubkey;
        let device_pubkey = logged_in.device_keys.public_key();
        let (session, response) = invite.accept_with_owner(
            device_pubkey,
            logged_in.device_keys.secret_key().to_secret_bytes(),
            Some(device_pubkey.to_hex()),
            Some(owner_pubkey),
        )?;
        let response_event = nostr_double_ratchet::invite_response_event(&response)?;
        self.publish_device_approval_result(&approval_relay_urls, receipt_event, response_event)?;
        let retry_batch = self
            .protocol_engine
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Protocol engine is not ready."))?
            .import_session_state(
                owner_pubkey,
                Some(invite.inviter_device_pubkey.to_hex()),
                session.state,
                unix_now(),
            )?;
        self.sync_local_app_keys_to_protocol_engine("device_approval");
        self.publish_local_protocol_invite();
        self.mark_mobile_push_dirty();
        self.process_protocol_engine_retry_batch("link_invite_import", retry_batch);
        Ok(())
    }
}
