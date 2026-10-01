use super::*;

impl AppCore {
    pub(in crate::core) fn private_sibling_control_waits_for_roster(
        &mut self,
        owner: PublicKey,
        sender: Option<PublicKey>,
        kind: u32,
    ) -> bool {
        if !matches!(kind, 10449 | 10450 | 10452 | 10453) {
            return false;
        }
        let Some(account) = self
            .logged_in
            .as_ref()
            .filter(|account| account.owner_pubkey == owner)
        else {
            // Foreign owners continue to the receiver's permanent rejection.
            return false;
        };
        let current = account.device_keys.public_key();
        let Some(sender) = sender.filter(|sender| *sender != current) else {
            return false;
        };
        let ready = self.device_sync_peer_is_authorized(&sender.to_hex())
            && self.protocol_engine.as_ref().is_some_and(|engine| {
                engine.owner_device_binding_is_verified(owner, current)
                    && engine.owner_device_binding_is_verified(owner, sender)
            });
        if ready {
            return false;
        }
        // A durable decrypted delivery can outlive the app's roster projection.
        // Keep its journal entry until both devices are authorized again.
        self.request_protocol_subscription_refresh();
        self.fetch_recent_protocol_metadata_state();
        true
    }
}
