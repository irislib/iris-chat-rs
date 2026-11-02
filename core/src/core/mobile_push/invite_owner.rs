use super::*;

const PREVIEW_PREFIX: &str = "mobile_push_invite:";

impl AppCore {
    pub(in crate::core) fn cache_invite_acceptance_preview(
        &mut self,
        event_id: &str,
        chat_id: &str,
    ) {
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|login| login.owner_pubkey.to_hex())
        else {
            return;
        };
        let save = || -> anyhow::Result<()> {
            let shared = self.app_store.shared();
            let mut conn = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
                rusqlite::params![
                    format!("{PREVIEW_PREFIX}{event_id}"),
                    serde_json::to_string(&(owner, chat_id))?
                ],
            )?;
            tx.execute("DELETE FROM app_meta WHERE key IN (SELECT key FROM app_meta WHERE key LIKE 'mobile_push_invite:%' ORDER BY rowid DESC LIMIT -1 OFFSET 256)", [])?;
            tx.commit()?;
            Ok(())
        };
        if let Err(error) = save() {
            self.push_debug_log("mobile_push.invite_preview.error", error.to_string());
        }
    }
}

pub(super) fn pending_invite_response_owner(
    data_dir: &str,
    owner_pubkey: PublicKey,
    device_keys: &Keys,
    event: &nostr::Event,
) -> Option<String> {
    let conn = open_lookup_connection(data_dir)?;
    super::super::storage::validate_account_storage(&conn, &owner_pubkey.to_hex()).ok()?;
    let enabled: bool = conn.query_row(
        "SELECT desktop_notifications_enabled AND invite_acceptance_notifications_enabled FROM preferences WHERE id = 1",
        [], |row| row.get(0),
    ).ok()?;
    if !enabled {
        return None;
    }
    let cached: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [format!("{PREVIEW_PREFIX}{}", event.id)],
            |row| row.get(0),
        )
        .optional()
        .ok()?;
    if let Some((owner, chat_id)) =
        cached.and_then(|json| serde_json::from_str::<(String, String)>(&json).ok())
    {
        return (owner == owner_pubkey.to_hex()
            && permits_invite_notification(&conn, owner_pubkey, &chat_id, event))
        .then_some(chat_id);
    }
    let shared_conn = Arc::new(Mutex::new(open_lookup_connection(data_dir)?));
    let storage = Arc::new(super::super::storage::SqliteStorageAdapter::new(
        shared_conn,
        owner_pubkey.to_hex(),
        device_keys.public_key().to_hex(),
    ));
    let device_secret = device_keys.secret_key().to_secret_bytes();
    // The protocol checkpoint owns the current invite. The legacy device-invite
    // record can be missing or stale after restoration or rotation.
    let mut preview_engine = direct_message_preview_engine(data_dir, owner_pubkey, device_keys)?;
    let mut invites = preview_engine
        .local_invite()
        .into_iter()
        .collect::<Vec<_>>();
    if let Ok(private_invites) = super::super::invites::load_private_chat_invites(storage.as_ref())
    {
        invites.extend(private_invites.into_values());
    }

    invites.into_iter().find_map(|mut invite| {
        // Re-decrypt for presentation even if the live runtime already consumed
        // this response. This copy never imports a session or persists anything.
        invite.used_response_contents.clear();
        let response =
            nostr_double_ratchet::process_invite_response_event(&invite, event, device_secret)
                .ok()
                .flatten()?;
        let authenticated_device = response.invitee_identity;
        let owner = match (response.owner_public_key, response.invitee_owner_pubkey) {
            (None, None) => authenticated_device,
            (Some(owner), Some(owner_claim)) if owner.to_bytes() == owner_claim.to_bytes() => {
                if owner != authenticated_device {
                    // Proofs travel encrypted inside the handshake. Ingest them
                    // only into the preview overlay, never the live ratchet store.
                    preview_engine
                        .ingest_invite_response_owner_proof(
                            &invite,
                            event,
                            owner,
                            authenticated_device,
                        )
                        .ok()?;
                    if !preview_engine
                        .invite_owner_device_is_authorized(owner, authenticated_device)
                    {
                        return None;
                    }
                }
                owner
            }
            _ => return None,
        };
        let chat_id = owner.to_hex();
        permits_invite_notification(&conn, owner_pubkey, &chat_id, event).then_some(chat_id)
    })
}

fn permits_invite_notification(
    conn: &rusqlite::Connection,
    owner: PublicKey,
    chat_id: &str,
    event: &Event,
) -> bool {
    chat_id != owner.to_hex()
        && !block_policy::suppresses(conn, chat_id, Some(event.created_at.as_secs()))
        && !is_chat_muted_in(conn, chat_id)
}
