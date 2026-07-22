use super::account::{device_approval_bootstrap_label, device_approval_pairing_invite};
use super::*;

impl AppCore {
    pub(super) fn start_linked_device(&mut self, _owner_input: &str) {
        self.push_debug_log("session.start_linked", "create device approval request");
        self.state.busy.linking_device = true;
        self.emit_state();

        if let Err(error) = self.create_pending_linked_device() {
            self.state.toast = Some(error.to_string());
        }

        self.state.busy.linking_device = false;
        self.screen_stack = vec![Screen::AddDevice];
        self.rebuild_state();
        self.emit_state();
    }

    fn create_pending_linked_device(&mut self) -> anyhow::Result<()> {
        self.stop_pending_linked_device();
        if self.device_approval_relay_urls.len() != 1 {
            anyhow::bail!("Device approval requires exactly one approval relay.");
        }

        let device_keys = Keys::generate();
        let device_pubkey = device_keys.public_key();
        let current_device_labels = self.current_device_labels.clone();
        let local_request = create_nostr_identity_device_approval_request(
            &device_keys,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: None,
                request_secret: None,
                requested_at: i64::try_from(unix_now().get()).unwrap_or(i64::MAX),
                request_type: Some("device_link".to_string()),
                resources: Vec::new(),
                expires_at: None,
                profile_id: None,
                admin_app_key_pubkey: None,
                label: current_device_labels
                    .as_ref()
                    .and_then(|labels| labels.device_label.as_deref())
                    .and_then(device_approval_bootstrap_label),
            },
        )?;
        let invite =
            device_approval_pairing_invite(device_pubkey, &local_request.request.request_secret)?;
        let bootstrap = nostr_identity_device_approval_bootstrap(&local_request.request)?;
        let url = encode_nostr_identity_device_approval_bootstrap(&bootstrap, None)?;
        self.install_pending_linked_device(device_keys, bootstrap, invite, url, true);
        Ok(())
    }

    pub(super) fn restore_pending_linked_device(
        &mut self,
        device_nsec: &str,
        approval_bootstrap_json: &str,
    ) {
        self.state.busy.restoring_session = true;
        self.emit_state();
        let result = (|| -> anyhow::Result<()> {
            let device_keys = Keys::parse(device_nsec.trim())?;
            let bootstrap: NostrIdentityDeviceApprovalBootstrap =
                serde_json::from_str(approval_bootstrap_json)?;
            if PublicKey::parse(&bootstrap.device_app_key_npub)? != device_keys.public_key() {
                anyhow::bail!("Stored device link does not match its secret key.");
            }
            let invite = device_approval_pairing_invite(
                device_keys.public_key(),
                &bootstrap.request_secret,
            )?;
            let url = encode_nostr_identity_device_approval_bootstrap(&bootstrap, None)?;
            self.stop_pending_linked_device();
            self.install_pending_linked_device(device_keys, bootstrap, invite, url, false);
            Ok(())
        })();
        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
            let _ = self.update_tx.send(AppUpdate::ClearPendingDeviceLink);
        }
        self.state.busy.restoring_session = false;
        self.screen_stack = vec![Screen::AddDevice];
        self.rebuild_state();
        self.emit_state();
    }
}
