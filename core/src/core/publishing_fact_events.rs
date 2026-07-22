use super::*;

const GROUP_ROSTER_PUBLISH_LABEL: &str = "group-roster";

impl AppCore {
    pub(super) fn publish_group_roster_fact(&mut self, group: &GroupSnapshot) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        let Some(owner_keys) = logged_in.owner_keys.as_ref() else {
            return false;
        };
        let local_owner =
            nostr_double_ratchet::OwnerPubkey::from_bytes(logged_in.owner_pubkey.to_bytes());
        if !group.admins.contains(&local_owner) {
            return false;
        }
        let unsigned = match nostr_double_ratchet::group_roster_unsigned_event(
            logged_in.owner_pubkey,
            group,
        ) {
            Ok(unsigned) => unsigned,
            Err(error) => {
                self.push_debug_log("group.roster_fact.publish", error.to_string());
                return false;
            }
        };
        let event = match unsigned.sign_with_keys(owner_keys) {
            Ok(event) => event,
            Err(error) => {
                self.push_debug_log("group.roster_fact.publish", error.to_string());
                return false;
            }
        };
        self.publish_runtime_event(event, GROUP_ROSTER_PUBLISH_LABEL, None)
    }

    pub(super) fn publish_local_app_keys_snapshot_only(&mut self, label: &'static str) -> bool {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return false;
        };
        if self.defer_owner_app_keys_publish {
            return false;
        }
        let Some(owner_keys) = logged_in.owner_keys.clone() else {
            return false;
        };
        let owner_pubkey = logged_in.owner_pubkey;
        let Some(local_app_keys) = self.app_keys.get(&owner_pubkey.to_hex()).cloned() else {
            return false;
        };
        let event = match known_app_keys_to_ndr(&local_app_keys)
            .get_encrypted_event_at(&owner_keys, local_app_keys.created_at_secs)
            .and_then(|unsigned| unsigned.sign_with_keys(&owner_keys).map_err(Into::into))
        {
            Ok(event) => event,
            Err(error) => {
                self.push_debug_log("publish.app_keys", error.to_string());
                return false;
            }
        };

        let published = self.publish_runtime_event(event, "app-keys", None);
        self.sync_local_app_keys_to_protocol_engine(label);
        published
    }
}
