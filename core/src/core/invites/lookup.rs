use super::*;

impl AppCore {
    pub(in crate::core) fn finish_private_invite_lookup(
        &mut self,
        request_id: String,
        owner: String,
        result: Result<crate::invite_record::InviteRecord, String>,
    ) {
        let Some((pending_id, acceptance_intent)) = self.pending_private_invite_lookup.as_ref()
        else {
            return;
        };
        if pending_id != &request_id
            || self
                .logged_in
                .as_ref()
                .map(|login| login.owner_pubkey.to_hex())
                .as_ref()
                != Some(&owner)
        {
            return;
        }
        let acceptance_intent = acceptance_intent.clone();
        self.pending_private_invite_lookup = None;
        match result.and_then(|record| {
            if record
                .expires_at
                .is_some_and(|expiry| expiry <= unix_now().get())
            {
                return Err("This invite has expired.".to_string());
            }
            record
                .legacy_url()
                .map(|url| (url, record))
                .map_err(|e| e.to_string())
        }) {
            Ok((url, record)) => {
                self.accept_invite_input(&url, Some(record), Some(acceptance_intent))
            }
            Err(error) => {
                self.state.busy.accepting_invite = false;
                self.state.toast = Some(error);
                self.emit_state();
            }
        }
    }
}
