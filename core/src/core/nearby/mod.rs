pub(crate) mod bitchat;

use super::*;

impl AppCore {
    pub(super) fn build_nearby_presence_event_json(
        &self,
        peer_id: &str,
        my_nonce: &str,
        their_nonce: &str,
        profile_event_id: &str,
    ) -> String {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return String::new();
        };
        let Some(owner_keys) = logged_in.owner_keys.as_ref() else {
            return String::new();
        };
        let peer_id = peer_id.trim();
        let my_nonce = my_nonce.trim();
        let their_nonce = their_nonce.trim();
        if peer_id.is_empty() || my_nonce.is_empty() || their_nonce.is_empty() {
            return String::new();
        }

        let expires_at = unix_now().get().saturating_add(120);
        let mut content = serde_json::json!({
            "protocol": "iris-nearby-v1",
            "transport": "ble",
            "peer_id": peer_id,
            "my_nonce": my_nonce,
            "their_nonce": their_nonce,
            "expires_at": expires_at,
        });
        let profile_event_id = profile_event_id.trim();
        if profile_event_id.len() == 64 {
            content["profile_event_id"] = serde_json::Value::String(profile_event_id.to_string());
        }

        let Ok(event) = EventBuilder::new(
            Kind::from(NEARBY_PRESENCE_KIND),
            serde_json::to_string(&content).unwrap_or_default(),
        )
        .sign_with_keys(owner_keys) else {
            return String::new();
        };

        serde_json::to_string(&event).unwrap_or_default()
    }
}
