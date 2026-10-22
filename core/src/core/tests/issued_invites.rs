// Exercise preparation, upload completion, projection, and admission through
// production code. Only the external upload acknowledgement is supplied here.
fn publish_personal_invite_for_test(core: &mut AppCore) {
    publish_issued_invite_for_test(core, None, None, None);
}

fn publish_issued_invite_for_test(core: &mut AppCore, group: Option<&str>, limit: Option<u32>, expires: Option<u64>) -> (String, super::issued_invites::IssuedInvite) {
    let (key, issued) = core.prepare_issued_invite(group, limit, expires).unwrap();
    core.ensure_local_app_keys_for_public_invite();
    let login = core.logged_in.as_ref().unwrap();
    let owner = login.owner_pubkey.to_hex();
    let device = login.device_keys.public_key().to_hex();
    let request = uuid::Uuid::new_v4().to_string();
    core.pending_invite_upload = Some((request.clone(), key.clone()));
    core.finish_invite_upload(request, owner, device, Ok(issued.url));
    assert!(core.state.toast.is_none(), "{:?}", core.state.toast);
    (key.clone(), core.issued_invites[&key].clone())
}

#[test]
fn native_private_invite_publication_is_acknowledged_private_and_account_scoped() {
    let owner = Keys::generate(); let device = Keys::generate();
    let mut core = logged_in_test_core("native-invite-publish", &owner, &device);
    let (key, issued) = core.prepare_issued_invite(None, None, None).unwrap();
    assert!(core.build_public_invite_snapshot().is_none());
    let (reference, ciphertext) = crate::invite_record::encode_record(&issued.record).unwrap();
    assert_eq!(issued.url, format!("https://chat.iris.to/#/{reference}"));
    let decoded = crate::invite_record::decode_record(&reference, &ciphertext).unwrap();
    assert_eq!(decoded.invite.owner, owner.public_key().to_hex());
    assert_eq!(decoded.invite.inviter, device.public_key().to_hex());
    assert!(decoded.token.is_none());
    assert!(!String::from_utf8_lossy(&ciphertext).contains("sharedSecret"));
    let id = "upload".to_string();
    core.pending_invite_upload = Some((id.clone(), key.clone()));
    core.finish_invite_upload(id.clone(), Keys::generate().public_key().to_hex(), device.public_key().to_hex(), Ok(issued.url.clone()));
    assert!(!core.issued_invites[&key].published);
    core.finish_invite_upload(id, owner.public_key().to_hex(), device.public_key().to_hex(), Ok(issued.url.clone()));
    let snapshot = core.build_public_invite_snapshot().unwrap();
    assert_eq!(snapshot.url, issued.url);
    assert!(snapshot.title.starts_with("Chat with "));
    assert!(super::invites::parse_public_invite_input(&snapshot.legacy_url).is_ok());
    let restored = super::issued_invites::load_issued_invites(&core.private_chat_invite_storage().unwrap()).unwrap();
    assert!(restored[&key].published);
}

#[test]
fn native_private_invite_failed_upload_does_not_expose_a_link() {
    let owner = Keys::generate(); let device = Keys::generate();
    let mut core = logged_in_test_core("native-invite-failed-upload", &owner, &device);
    let (key, _) = core.prepare_issued_invite(None, None, None).unwrap();
    core.pending_invite_upload = Some(("upload".into(), key));
    core.finish_invite_upload("upload".into(), owner.public_key().to_hex(), device.public_key().to_hex(), Err("Offline".into()));
    assert!(core.build_public_invite_snapshot().is_none());
    assert!(!core.state.busy.creating_invite);
    assert_eq!(core.state.toast.as_deref(), Some("Offline"));
}

#[test]
fn native_group_invite_limits_revocation_expiry_and_removed_member_reuse() {
    let owner = Keys::generate(); let device = Keys::generate();
    let mut core = logged_in_test_core("native-group-invite", &owner, &device);
    core.create_group("Picnic", &[]);
    let group_id = core.groups.keys().next().unwrap().clone();
    let (key, issued) = publish_issued_invite_for_test(&mut core, Some(&group_id), Some(1), Some(30 * 86400));
    let alice = Keys::generate().public_key(); let bob = Keys::generate().public_key();
    assert!(issued.record.legacy_url().unwrap().contains("sharedSecret"));
    assert_eq!(core.admit_invite_request(&issued, alice).unwrap(), "added");
    assert_eq!(core.admit_invite_request(&issued, alice).unwrap(), "already-member");
    assert_eq!(core.admit_invite_request(&issued, bob).unwrap(), "full");
    assert_eq!(core.group_invite_snapshots(&group_id)[0].use_count, 1);
    assert!(!core.threads.contains_key(&alice.to_hex()), "A group join must not create an empty DM");
    // Restore the protocol engine to prove both the member and use ledger survived.
    let storage = Arc::new(core.private_chat_invite_storage().unwrap());
    let restored = ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device).unwrap();
    assert_eq!(restored.group_invite_use_count(&group_id, issued.record.token.as_ref().unwrap()), 1);
    core.protocol_engine = Some(restored);
    core.remove_group_member(&group_id, &alice.to_hex());
    assert_eq!(core.admit_invite_request(&issued, alice).unwrap(), "used");
    let mut expired = issued.clone(); expired.record.expires_at = Some(1);
    assert_eq!(core.admit_invite_request(&expired, bob).unwrap(), "expired");
    core.revoke_group_invite(&group_id, &key);
    let revoked = core.issued_invites[&key].clone();
    assert_eq!(core.admit_invite_request(&revoked, bob).unwrap(), "unavailable");
    assert!(core.group_invite_snapshots(&group_id).is_empty());
}

#[test]
fn native_group_invite_requires_current_admin_and_authenticated_controls() {
    let owner = Keys::generate(); let device = Keys::generate();
    let mut core = logged_in_test_core("native-invite-admin", &owner, &device);
    core.create_group("Picnic", &[]);
    let group_id = core.groups.keys().next().unwrap().clone();
    let (_, issued) = publish_issued_invite_for_test(&mut core, Some(&group_id), None, None);
    let peer = Keys::generate().public_key();
    let request = serde_json::json!({"v":1,"groupId":group_id,"token":issued.record.token}).to_string();
    assert!(core.receive_group_invite_request(peer, None, &request));
    assert_eq!(core.group_invite_snapshots(&group_id)[0].use_count, 0);
    for content in ["null", "[]", "{}", "{\"v\":1}"] {
        assert!(core.receive_group_invite_request(peer, Some(peer), content));
    }
    core.groups.get_mut(&group_id).unwrap().admins.clear();
    assert_eq!(core.admit_invite_request(&issued, peer).unwrap(), "unavailable");
    assert!(core.prepare_issued_invite(Some(&group_id), None, None).is_err());
}

#[test]
fn native_group_invite_retry_repairs_interrupted_membership_projection() {
    let owner = Keys::generate(); let device = Keys::generate();
    let mut core = logged_in_test_core("native-invite-projection-retry", &owner, &device);
    core.create_group("Picnic", &[]);
    let group_id = core.groups.keys().next().unwrap().clone();
    let (_, issued) = publish_issued_invite_for_test(&mut core, Some(&group_id), Some(1), None);
    let peer = Keys::generate().public_key();
    let token = issued.record.token.as_ref().unwrap();
    core.protocol_engine.as_mut().unwrap().admit_group_invite_member(&group_id, token, peer, Some(1)).unwrap();
    // Crash after the atomic engine commit, before updating app-state groups.
    assert!(!core.groups[&group_id].members.contains(&NdrOwnerPubkey::from_bytes(peer.to_bytes())));
    let storage = Arc::new(core.private_chat_invite_storage().unwrap());
    core.protocol_engine = Some(ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device).unwrap());
    assert_eq!(core.admit_invite_request(&issued, peer).unwrap(), "already-member");
    assert_eq!(core.group_invite_snapshots(&group_id)[0].use_count, 1);
    assert!(core.groups[&group_id].members.contains(&NdrOwnerPubkey::from_bytes(peer.to_bytes())));
}

#[test]
fn native_group_invite_storage_failure_rolls_back_membership_and_use_together() {
    let owner = Keys::generate(); let device = Keys::generate();
    let mut core = logged_in_test_core("native-invite-atomic", &owner, &device);
    core.create_group("Picnic", &[]);
    let group_id = core.groups.keys().next().unwrap().clone();
    let (_, issued) = publish_issued_invite_for_test(&mut core, Some(&group_id), Some(1), None);
    let peer = Keys::generate().public_key();
    let connection = core.app_store.shared();
    connection.lock().unwrap().execute_batch(
        "CREATE TEMP TRIGGER fail_invite_insert BEFORE INSERT ON ndr_kv BEGIN SELECT RAISE(ABORT, 'injected admission failure'); END;
         CREATE TEMP TRIGGER fail_invite_update BEFORE UPDATE ON ndr_kv BEGIN SELECT RAISE(ABORT, 'injected admission failure'); END;"
    ).unwrap();
    assert!(core.admit_invite_request(&issued, peer).is_err());
    assert_eq!(core.group_invite_snapshots(&group_id)[0].use_count, 0);
    assert!(!core.groups[&group_id].members.contains(&NdrOwnerPubkey::from_bytes(peer.to_bytes())));
    connection.lock().unwrap().execute_batch("DROP TRIGGER fail_invite_insert; DROP TRIGGER fail_invite_update;").unwrap();
    assert_eq!(core.admit_invite_request(&issued, peer).unwrap(), "added");
    assert_eq!(core.group_invite_snapshots(&group_id)[0].use_count, 1);
}

impl AppCore {
    pub(super) fn create_legacy_invite_for_test(&mut self) {
        if !self.can_use_chats() {
            self.state.toast = Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
            self.emit_state();
            return;
        }

        self.state.busy.creating_invite = true;
        self.emit_state();

        let result = (|| -> anyhow::Result<Invite> {
            let logged_in = self
                .logged_in
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Create or restore a profile first."))?;
            let device_pubkey = logged_in.device_keys.public_key();
            let device_id = device_pubkey.to_hex();
            let mut invite = Invite::create_new(device_pubkey, Some(device_id), Some(1))?;
            invite.owner_public_key = Some(logged_in.owner_pubkey);
            invite.purpose = Some("private".to_string());
            Ok(invite)
        })();

        match result {
            Ok(invite) => {
                self.ensure_local_app_keys_for_public_invite();
                if let Err(error) = self.store_private_chat_invite(&invite) {
                    self.state.toast = Some(error.to_string());
                } else {
                    self.private_chat_invites
                        .insert(super::invites::private_chat_invite_key(&invite), invite);
                    self.mark_mobile_push_dirty();
                    self.request_protocol_subscription_refresh();
                    self.persist_best_effort();
                }
            }
            Err(error) => {
                self.state.toast = Some(error.to_string());
            }
        }

        self.state.busy.creating_invite = false;
        self.rebuild_state();
        self.emit_state();
    }

}

#[test]
fn native_personal_invite_rewrapped_response_cannot_reset_a_session() {
    let owner = Keys::generate(); let device = Keys::generate();
    let peer_owner = Keys::generate(); let peer_device = Keys::generate();
    let mut core = logged_in_test_core("native-reusable-replay", &owner, &device);
    publish_personal_invite_for_test(&mut core);
    let invite = super::invites::parse_public_invite_input(&core.state.public_invite.as_ref().unwrap().legacy_url).unwrap();
    let (_, mut envelope) = invite.accept_with_owner(peer_device.public_key(), peer_device.secret_key().to_secret_bytes(), Some(peer_device.public_key().to_hex()), Some(peer_owner.public_key())).unwrap();
    core.handle_relay_event(invite_response_event(&envelope).unwrap());
    prove_invite_owner(&mut core, &peer_owner, &peer_device, 10);
    assert_eq!(core.protocol_engine.as_ref().unwrap().active_session_count_for_owner(peer_owner.public_key()), 1);
    core.send_message(&peer_owner.public_key().to_hex(), "advance the ratchet", None);
    let before = serde_json::to_value(core.protocol_engine.as_ref().unwrap().session_manager_snapshot_for_test()).unwrap();
    envelope.created_at = NdrUnixSeconds(envelope.created_at.get() + 1);
    core.handle_relay_event(invite_response_event(&envelope).unwrap());
    let after = serde_json::to_value(core.protocol_engine.as_ref().unwrap().session_manager_snapshot_for_test()).unwrap();
    assert_eq!(after, before, "A new outer wrapper must not reinstall the old session");
    assert!(core.build_public_invite_snapshot().unwrap().url.contains("/#/nhash1"));
}
