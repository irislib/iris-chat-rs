use super::*;

impl ProtocolEngine {
    /// Message authors of the accepted owners, most important first: the live
    /// ratchet keys of active sessions, then of inactive ones, then keys that
    /// only still have missed messages from older ratchet periods. A
    /// subscription that reaches a relay limit drops from the end.
    pub fn message_author_pubkeys_by_priority<F>(&self, accept_owner: F) -> Vec<PublicKey>
    where
        F: Fn(PublicKey) -> bool,
    {
        let snapshot = self.session_manager.snapshot();
        let mut active = Vec::new();
        let mut inactive = Vec::new();
        for user in &snapshot.users {
            let Ok(owner) = PublicKey::parse(&user.owner_pubkey.to_string()) else {
                continue;
            };
            if !accept_owner(owner) {
                continue;
            }
            for device in &user.devices {
                if !device.authorized
                    || device.is_stale
                    || !self.owner_device_binding_is_verified_in_roster(
                        user.owner_pubkey,
                        device.device_pubkey,
                        user.roster.as_ref(),
                    )
                {
                    continue;
                }
                active.extend(&device.active_session);
                inactive.extend(&device.inactive_sessions);
            }
        }
        let sessions = || active.iter().chain(&inactive).copied();
        let mut ranked = nostr_double_ratchet::prioritized_message_authors(sessions());
        // The library ranks current-format sessions. Keep every other author
        // the unordered projection subscribes to, after them.
        let mut all = HashSet::new();
        for session in sessions() {
            collect_expected_sender_pubkeys(session, &mut all);
        }
        for author in &ranked {
            all.remove(author);
        }
        let mut rest = all.into_iter().collect::<Vec<_>>();
        rest.sort_unstable();
        ranked.extend(rest);
        ranked
    }
}
