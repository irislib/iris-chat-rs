use super::*;
use crate::invite_record::{InviteConnection, InviteRecord};
use crate::state::GroupInviteSnapshot;

const PREFIX: &str = "issued-private-invites-v1/";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct IssuedInvite {
    pub record: InviteRecord,
    pub url: String,
    pub created_at_ms: u64,
    pub published: bool,
    pub revoked: bool,
}

fn bounded_name(value: &str) -> String {
    let mut units = 0;
    value
        .trim()
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 100
        })
        .collect()
}

fn random_secret() -> String {
    hashtree_core::to_hex(&rand::random::<[u8; 32]>())
}

pub(super) fn load_issued_invites(
    storage: &dyn StorageAdapter,
) -> anyhow::Result<BTreeMap<String, IssuedInvite>> {
    let mut records = BTreeMap::new();
    for key in storage.list(PREFIX)? {
        let Some(value) = storage.get(&key)? else {
            continue;
        };
        let record: IssuedInvite = serde_json::from_str(&value)?;
        record.record.validate()?;
        records.insert(key.trim_start_matches(PREFIX).to_string(), record);
    }
    Ok(records)
}

impl AppCore {
    pub(super) fn create_public_invite(&mut self) {
        self.begin_invite_upload(None, None, None);
    }

    pub(super) fn create_group_invite(
        &mut self,
        group_id: &str,
        max_uses: Option<u32>,
        expires_in_seconds: Option<u64>,
    ) {
        self.begin_invite_upload(Some(group_id), max_uses, expires_in_seconds);
    }

    fn local_group_admin(&self, group_id: &str) -> bool {
        self.logged_in
            .as_ref()
            .zip(self.groups.get(group_id))
            .is_some_and(|(login, group)| {
                let owner = NdrOwnerPubkey::from_bytes(login.owner_pubkey.to_bytes());
                group.admins.contains(&owner) && group.members.contains(&owner)
            })
    }

    fn save_issued_invite(&self, key: &str, issued: &IssuedInvite) -> anyhow::Result<()> {
        self.private_chat_invite_storage()?
            .put(&format!("{PREFIX}{key}"), serde_json::to_string(issued)?)?;
        Ok(())
    }

    // Preparation is synchronous and durable; publication is acknowledged asynchronously.
    pub(super) fn prepare_issued_invite(
        &mut self,
        group_id: Option<&str>,
        max_uses: Option<u32>,
        expires_in_seconds: Option<u64>,
    ) -> anyhow::Result<(String, IssuedInvite)> {
        anyhow::ensure!(self.can_use_chats(), "Create or restore a profile first.");
        anyhow::ensure!(
            max_uses.is_none_or(|n| (1..=10000).contains(&n)),
            "Invalid use limit."
        );
        anyhow::ensure!(
            expires_in_seconds.is_none_or(|n| n > 0 && n <= 365 * 24 * 60 * 60),
            "Invalid expiry."
        );
        if let Some(id) = group_id {
            anyhow::ensure!(
                self.local_group_admin(id),
                "Only group admins can create invite links."
            );
        }
        let login = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not logged in"))?;
        let device = login.device_keys.public_key();
        let mut invite = Invite::create_new(device, Some(device.to_hex()), None)?;
        invite.owner_public_key = Some(login.owner_pubkey);
        invite.purpose = Some("private".to_string());
        // Only the public bootstrap fields go in the encrypted record.
        let connection = InviteConnection {
            inviter: device.to_hex(),
            ephemeral_key: invite.inviter_ephemeral_public_key.to_string(),
            shared_secret: hashtree_core::to_hex(&invite.shared_secret),
            owner: login.owner_pubkey.to_hex(),
            device_id: invite.device_id.clone(),
            purpose: invite.purpose.clone(),
        };
        let name = group_id
            .and_then(|id| self.groups.get(id))
            .map(|g| g.name.clone())
            .unwrap_or_else(|| {
                self.state
                    .account
                    .as_ref()
                    .map(|a| a.display_name.clone())
                    .unwrap_or_default()
            });
        let record = InviteRecord {
            v: 1,
            kind: if group_id.is_some() {
                "group"
            } else {
                "personal"
            }
            .into(),
            name: bounded_name(&name),
            nonce: random_secret(),
            invite: connection,
            group_id: group_id.map(str::to_owned),
            token: group_id.map(|_| random_secret()),
            expires_at: expires_in_seconds.map(|seconds| unix_now().get().saturating_add(seconds)),
            max_uses,
        };
        let (reference, _) = crate::invite_record::encode_record(&record)?;
        let issued = IssuedInvite {
            record,
            url: format!("https://chat.iris.to/#/{reference}"),
            created_at_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
            published: false,
            revoked: false,
        };
        let key = super::invites::private_chat_invite_key(&invite);
        self.store_private_chat_invite(&invite)?;
        self.save_issued_invite(&key, &issued)?;
        self.private_chat_invites.insert(key.clone(), invite);
        self.issued_invites.insert(key.clone(), issued.clone());
        self.ensure_local_app_keys_for_public_invite();
        self.mark_mobile_push_dirty();
        self.request_protocol_subscription_refresh();
        Ok((key, issued))
    }

    fn begin_invite_upload(
        &mut self,
        group_id: Option<&str>,
        max_uses: Option<u32>,
        expires_in_seconds: Option<u64>,
    ) {
        if self.pending_invite_upload.is_some() {
            return;
        }
        let (key, issued) = match self.prepare_issued_invite(group_id, max_uses, expires_in_seconds)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                self.state.toast = Some(error.to_string());
                self.emit_state();
                return;
            }
        };
        let Some(login) = self.logged_in.as_ref() else {
            return;
        };
        let owner = login.owner_pubkey.to_hex();
        let device = login.device_keys.public_key().to_hex();
        let secret = login
            .owner_keys
            .as_ref()
            .unwrap_or(&login.device_keys)
            .secret_key()
            .to_secret_hex();
        let request_id = uuid::Uuid::new_v4().to_string();
        self.pending_invite_upload = Some((request_id.clone(), key));
        self.state.busy.creating_invite = true;
        self.rebuild_state();
        self.emit_state();
        let sender = self.core_sender.clone();
        self.runtime.spawn(async move {
            let result = super::attachment_upload::upload_invite_record(&secret, &issued.record)
                .await
                .map_err(|_| {
                    "Could not create invite. Check your connection and try again.".to_string()
                });
            let _ = sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::PrivateInviteUploaded {
                    request_id,
                    owner,
                    device,
                    result,
                },
            )));
        });
    }

    pub(super) fn finish_invite_upload(
        &mut self,
        request_id: String,
        owner: String,
        device: String,
        result: Result<String, String>,
    ) {
        let Some((pending_id, key)) = self.pending_invite_upload.clone() else {
            return;
        };
        if pending_id != request_id
            || !self.logged_in.as_ref().is_some_and(|l| {
                l.owner_pubkey.to_hex() == owner && l.device_keys.public_key().to_hex() == device
            })
        {
            return;
        }
        self.pending_invite_upload = None;
        self.state.busy.creating_invite = false;
        let result = (|| -> anyhow::Result<()> {
            let url = result.map_err(anyhow::Error::msg)?;
            let mut issued = self
                .issued_invites
                .get(&key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Invite unavailable."))?;
            anyhow::ensure!(url == issued.url && !issued.revoked, "Invite unavailable.");
            if let Some(group_id) = &issued.record.group_id {
                anyhow::ensure!(
                    self.local_group_admin(group_id),
                    "You’re no longer a group admin."
                );
            }
            issued.published = true;
            self.save_issued_invite(&key, &issued)?;
            self.issued_invites.insert(key, issued);
            Ok(())
        })();
        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        }
        self.rebuild_persist_and_emit_state();
    }

    fn displayed_personal_invite(&self) -> Option<(&String, &IssuedInvite)> {
        self.issued_invites
            .iter()
            .filter(|(_, i)| i.published && !i.revoked && i.record.kind == "personal")
            .max_by_key(|(key, i)| (i.created_at_ms, *key))
    }

    pub(super) fn is_displayed_personal_invite(&self, key: &str) -> bool {
        self.displayed_personal_invite()
            .is_some_and(|(shown, _)| shown == key)
    }

    pub(super) fn personal_invite_snapshot(&self) -> Option<PublicInviteSnapshot> {
        let (key, issued) = self.displayed_personal_invite()?;
        let name = self
            .state
            .account
            .as_ref()
            .map(|a| a.display_name.as_str())
            .unwrap_or(&issued.record.name)
            .trim();
        Some(PublicInviteSnapshot {
            url: issued.url.clone(),
            legacy_url: super::invites::chat_invite_url(self.private_chat_invites.get(key)?)
                .ok()?,
            title: if name.is_empty() {
                "Chat with me".into()
            } else {
                format!("Chat with {name}")
            },
            open_chat_on_join: self.open_invite_chat_on_join,
        })
    }

    pub(super) fn group_invite_snapshots(&self, group_id: &str) -> Vec<GroupInviteSnapshot> {
        if !self.local_group_admin(group_id) {
            return Vec::new();
        }
        self.issued_invites
            .iter()
            .filter(|(_, i)| {
                i.published && !i.revoked && i.record.group_id.as_deref() == Some(group_id)
            })
            .map(|(key, i)| GroupInviteSnapshot {
                id: key.clone(),
                url: i.url.clone(),
                title: format!(
                    "Join {}",
                    self.groups
                        .get(group_id)
                        .map(|g| g.name.as_str())
                        .unwrap_or(&i.record.name)
                ),
                use_count: self.protocol_engine.as_ref().map_or(0, |e| {
                    e.group_invite_use_count(
                        group_id,
                        i.record.token.as_deref().unwrap_or_default(),
                    )
                }) as u32,
                max_uses: i.record.max_uses,
                expires_at: i.record.expires_at,
            })
            .collect()
    }

    pub(super) fn revoke_group_invite(&mut self, group_id: &str, invite_id: &str) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                self.local_group_admin(group_id),
                "Only group admins can revoke invite links."
            );
            let mut issued = self
                .issued_invites
                .get(invite_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Invite unavailable."))?;
            anyhow::ensure!(
                issued.record.group_id.as_deref() == Some(group_id),
                "Invite unavailable."
            );
            issued.revoked = true;
            self.save_issued_invite(invite_id, &issued)?;
            self.issued_invites.insert(invite_id.to_owned(), issued);
            Ok(())
        })();
        if let Err(error) = result {
            self.state.toast = Some(error.to_string());
        }
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn receive_group_invite_request(
        &mut self,
        sender: PublicKey,
        device: Option<PublicKey>,
        content: &str,
    ) -> bool {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Request {
            v: u8,
            group_id: String,
            token: String,
        }
        if content.len() > 1024 || device.is_none() || self.is_owner_blocked(&sender.to_hex()) {
            return true;
        }
        let Ok(request) = serde_json::from_str::<Request>(content) else {
            return true;
        };
        if request.v != 1
            || request.group_id.encode_utf16().count() > 128
            || request.token.len() != 64
        {
            return true;
        }
        let issued = self
            .issued_invites
            .values()
            .find(|i| {
                i.record.group_id.as_deref() == Some(&request.group_id)
                    && i.record.token.as_deref() == Some(&request.token)
            })
            .cloned();
        let Some(issued) = issued else { return true };
        let status = self.admit_invite_request(&issued, sender);
        let Ok(status) = status else {
            // Retain the durable decrypted delivery for retry on storage failure.
            return false;
        };
        let Some(login) = self.logged_in.as_ref() else {
            return false;
        };
        let content = serde_json::json!({"v": 1, "groupId": request.group_id, "token": request.token, "status": status}).to_string();
        let unsigned = EventBuilder::new(Kind::Custom(10462), content)
            .tag(nostr::Tag::public_key(sender))
            .build(login.owner_pubkey);
        let sent = self.send_protocol_engine_unsigned_event_to_peer_only(
            sender,
            &sender.to_hex(),
            unsigned,
            "group_invite.result",
        );
        self.schedule_fast_protocol_retry_if_pending();
        self.rebuild_persist_and_emit_state();
        sent
    }

    pub(super) fn admit_invite_request(
        &mut self,
        issued: &IssuedInvite,
        sender: PublicKey,
    ) -> anyhow::Result<&'static str> {
        let record = &issued.record;
        let group_id = record
            .group_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Invalid group invite"))?;
        let token = record
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Invalid group invite"))?;
        // Admission and its use receipt commit before the UI projection. Repair
        // an interrupted projection before deciding whether this is a retry.
        let durable_group = self
            .protocol_engine
            .as_ref()
            .and_then(|engine| engine.group_snapshot(group_id));
        if let Some(group) = durable_group {
            let previous = self.groups.get(group_id).cloned();
            if previous.as_ref().is_none_or(|old| {
                (old.revision, old.updated_at) < (group.revision, group.updated_at)
            }) {
                self.apply_local_group_snapshot(previous.as_ref(), group, "group.invite_restored");
            }
        }
        let correct_device = self.logged_in.as_ref().is_some_and(|l| {
            l.owner_pubkey.to_hex() == record.invite.owner
                && l.device_keys.public_key().to_hex() == record.invite.inviter
        });
        if issued.revoked
            || !issued.published
            || !correct_device
            || !self.local_group_admin(group_id)
        {
            return Ok("unavailable");
        }
        if record
            .expires_at
            .is_some_and(|expiry| expiry <= unix_now().get())
        {
            return Ok("expired");
        }
        let previous = self
            .groups
            .get(group_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Group unavailable"))?;
        let member = NdrOwnerPubkey::from_bytes(sender.to_bytes());
        let engine = self
            .protocol_engine
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Protocol not ready"))?;
        if previous.members.contains(&member) {
            return Ok("already-member");
        }
        if engine.group_invite_used_by(group_id, token, sender) {
            return Ok("used");
        }
        if record
            .max_uses
            .is_some_and(|limit| engine.group_invite_use_count(group_id, token) >= limit as usize)
        {
            return Ok("full");
        }
        let result = engine.admit_group_invite_member(group_id, token, sender, record.max_uses)?;
        self.process_protocol_engine_effects(result.effects);
        if let Some(snapshot) = result.snapshot {
            self.apply_local_group_snapshot(Some(&previous), snapshot, "group.invite_admitted");
        }
        Ok("added")
    }
}
