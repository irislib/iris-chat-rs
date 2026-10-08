use super::*;

impl AppCore {
    // None dispatches the control; Some(false) retains it; Some(true) rejects it.
    pub(in crate::core) fn private_sibling_control_disposition(
        &mut self,
        owner: PublicKey,
        sender: Option<PublicKey>,
        rumor: &RuntimeRumor,
    ) -> Option<bool> {
        if !matches!(rumor.kind, 10449 | 10450 | 10452 | 10453 | 10454) {
            return None;
        }
        let Some(account) = self
            .logged_in
            .as_ref()
            .filter(|account| account.owner_pubkey == owner)
        else {
            return Some(true);
        };
        let owner_hex = owner.to_hex();
        let mut recipients = rumor
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().is_some_and(|name| name == "p"));
        // The original native builder normalized p=author away. Permit that
        // historical form only with the verified sibling authority checked below.
        if recipients
            .next()
            .is_some_and(|tag| tag.as_slice() != ["p", owner_hex.as_str()])
            || recipients.next().is_some()
        {
            return Some(true);
        }
        let current = account.device_keys.public_key();
        let Some(sender) = sender.filter(|sender| *sender != current) else {
            return Some(true);
        };
        if !self.protocol_engine.as_ref().is_some_and(|engine| {
            engine.owner_device_binding_is_verified(owner, current)
                && engine.owner_device_binding_is_verified(owner, sender)
        }) {
            // The engine persists authenticated deliveries with their protocol
            // state. Missing protocol authority cannot be repaired by an older
            // app projection, including after a signed device removal.
            return Some(true);
        }
        if self.device_sync_peer_is_authorized(&sender.to_hex()) {
            return None;
        }
        // A durable decrypted delivery can outlive the app's roster projection.
        // Keep its journal entry until both devices are authorized again.
        self.request_protocol_subscription_refresh();
        self.fetch_recent_protocol_metadata_state();
        Some(false)
    }
}
