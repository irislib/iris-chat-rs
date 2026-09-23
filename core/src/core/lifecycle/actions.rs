use super::*;

impl AppCore {
    pub(in crate::core) fn handle_action(&mut self, action: AppAction) {
        self.state.toast = None;
        match action {
            AppAction::StartCall { chat_id, video } => self.start_call(&chat_id, video),
            AppAction::AnswerCall { call_id } => self.answer_call(&call_id, false),
            AppAction::AnswerCallWithVoice { call_id } => self.answer_call(&call_id, true),
            AppAction::EndCall { call_id } => self.end_call(&call_id),
            AppAction::SetCallMuted { muted } => self.set_call_muted(muted),
            AppAction::SetCallVideoEnabled { enabled } => self.set_call_video(enabled),
            AppAction::SendCallMedia {
                call_id,
                kind,
                timestamp_us,
                key_frame,
                data,
            } => self.send_call_media(&call_id, kind, timestamp_us, key_frame, data),
            AppAction::RequestCallKeyFrame { call_id } => self.request_call_key_frame(&call_id),
            AppAction::SetCallMediaConnected { call_id, connected } => {
                self.set_call_media_connected(&call_id, connected)
            }
            AppAction::SetCallQuality {
                quality,
                max_bitrate_bps,
            } => self.set_call_quality(quality, max_bitrate_bps),
            AppAction::SetVoiceCallsEnabled { enabled } => self.set_calls_enabled(false, enabled),
            AppAction::SetVideoCallsEnabled { enabled } => self.set_calls_enabled(true, enabled),
            AppAction::CreateAccount { name } => self.create_account(&name),
            AppAction::UpdateProfileMetadata {
                name,
                picture_url,
                about,
            } => self.update_profile_metadata(&name, picture_url.as_deref(), about.as_deref()),
            AppAction::SetContactNickname {
                owner_pubkey_hex,
                nickname,
            } => self.set_contact_nickname(&owner_pubkey_hex, &nickname),
            AppAction::DeleteProfileMetadata => self.delete_profile_metadata(),
            AppAction::RestoreSession { owner_nsec } => self.restore_primary_session(&owner_nsec),
            AppAction::RestoreAccountBundle {
                owner_nsec,
                owner_pubkey_hex,
                device_nsec,
            } => self.restore_account_bundle(owner_nsec, &owner_pubkey_hex, &device_nsec),
            AppAction::RestorePendingDeviceLink {
                device_nsec,
                approval_bootstrap_json,
            } => self.restore_pending_linked_device(&device_nsec, &approval_bootstrap_json),
            AppAction::StartRemoteSignerLogin => self.start_remote_signer_login(None),
            AppAction::ConnectRemoteSigner { connection_uri } => {
                self.start_remote_signer_login(Some(&connection_uri))
            }
            AppAction::CancelRemoteSignerLogin => {
                self.cancel_remote_signer_login();
                self.emit_state();
            }
            AppAction::BeginSignerLogin { owner_pubkey_hex } => {
                self.cancel_remote_signer_login();
                self.begin_signer_login(&owner_pubkey_hex)
            }
            AppAction::CompleteSignerLogin {
                request_id,
                signed_event_json,
            } => self.complete_signer_login(&request_id, &signed_event_json),
            AppAction::CancelSignerLogin { request_id } => self.cancel_signer_login(&request_id),
            AppAction::StartLinkedDevice { owner_input } => self.start_linked_device(&owner_input),
            AppAction::SetCurrentDeviceLabels {
                device_label,
                client_label,
            } => self.set_current_device_labels(&device_label, &client_label),
            AppAction::AppForegrounded => self.handle_app_foregrounded(),
            AppAction::Logout => self.logout(),
            AppAction::CreateChat { peer_input } => self.create_chat(&peer_input),
            AppAction::CreateGroup {
                name,
                member_inputs,
            } => self.create_group(&name, &member_inputs),
            AppAction::CreateGroupWithPicture {
                name,
                member_inputs,
                picture_file_path,
                picture_filename,
            } => self.create_group_with_picture(
                &name,
                &member_inputs,
                &picture_file_path,
                &picture_filename,
            ),
            AppAction::CreatePublicInvite => self.create_public_invite(),
            AppAction::AcceptInvite { invite_input } => self.accept_invite(&invite_input),
            AppAction::OpenChat { chat_id } => self.open_chat(&chat_id),
            AppAction::RetryDirectChatCapability { chat_id } => {
                self.retry_direct_chat_capability(&chat_id)
            }
            AppAction::SendMessage { chat_id, text } => self.send_message(&chat_id, &text, None),
            AppAction::SendDisappearingMessage {
                chat_id,
                text,
                expires_at_secs,
            } => self.send_message(&chat_id, &text, Some(expires_at_secs)),
            AppAction::SetChatMessageTtl {
                chat_id,
                ttl_seconds,
            } => self.set_chat_message_ttl(&chat_id, ttl_seconds),
            AppAction::SetChatMuted { chat_id, muted } => self.set_chat_muted(&chat_id, muted),
            AppAction::SetChatPinned { chat_id, pinned } => self.set_chat_pinned(&chat_id, pinned),
            AppAction::SetChatUnread { chat_id, unread } => self.set_chat_unread(&chat_id, unread),
            AppAction::SendAttachment {
                chat_id,
                file_path,
                filename,
                caption,
            } => self.send_attachment(&chat_id, &file_path, &filename, &caption),
            AppAction::SendAttachments {
                chat_id,
                attachments,
                caption,
            } => self.send_attachments(&chat_id, &attachments, &caption),
            AppAction::ToggleReaction {
                chat_id,
                message_id,
                emoji,
            } => self.toggle_reaction(&chat_id, &message_id, &emoji),
            AppAction::SendTyping { chat_id } => self.send_typing(&chat_id),
            AppAction::StopTyping { chat_id } => self.stop_typing(&chat_id),
            AppAction::SetTypingIndicatorsEnabled { enabled } => {
                self.set_typing_indicators_enabled(enabled)
            }
            AppAction::SetReadReceiptsEnabled { enabled } => {
                self.set_read_receipts_enabled(enabled)
            }
            AppAction::SetDesktopNotificationsEnabled { enabled } => {
                self.set_desktop_notifications_enabled(enabled)
            }
            AppAction::SetInviteAcceptanceNotificationsEnabled { enabled } => {
                self.set_invite_acceptance_notifications_enabled(enabled)
            }
            AppAction::SetStartupAtLoginEnabled { enabled } => {
                self.set_startup_at_login_enabled(enabled)
            }
            AppAction::SetNearbyEnabled { enabled } => self.set_nearby_enabled(enabled),
            AppAction::SetNearbyBluetoothEnabled { enabled } => {
                self.set_nearby_bluetooth_enabled(enabled)
            }
            AppAction::SetNearbyLanEnabled { enabled } => self.set_nearby_lan_enabled(enabled),
            AppAction::SetDebugLoggingEnabled { enabled } => {
                self.set_debug_logging_enabled(enabled)
            }
            AppAction::SetAcceptUnknownDirectMessages { enabled } => {
                self.set_accept_unknown_direct_messages(enabled)
            }
            AppAction::SetUserBlocked {
                owner_pubkey_hex,
                blocked,
            } => self.set_user_blocked(&owner_pubkey_hex, blocked),
            AppAction::SetMessageRequestAccepted { chat_id } => {
                self.accept_message_request(&chat_id)
            }
            AppAction::SetNearbyMailbagEnabled { enabled } => {
                self.set_nearby_mailbag_enabled(enabled)
            }
            AppAction::SetNearbyShowInChatList { enabled } => {
                self.set_nearby_show_in_chat_list(enabled)
            }
            AppAction::AddNostrRelay { relay_url } => self.add_nostr_relay(&relay_url),
            AppAction::UpdateNostrRelay {
                old_relay_url,
                new_relay_url,
            } => self.update_nostr_relay(&old_relay_url, &new_relay_url),
            AppAction::RemoveNostrRelay { relay_url } => self.remove_nostr_relay(&relay_url),
            AppAction::SetNostrRelays { relay_urls } => self.set_nostr_relays(&relay_urls),
            AppAction::ResetNostrRelays => self.reset_nostr_relays(),
            AppAction::SetImageProxyEnabled { enabled } => self.set_image_proxy_enabled(enabled),
            AppAction::SetImageProxyFallbackEnabled { enabled } => {
                self.set_image_proxy_fallback_enabled(enabled)
            }
            AppAction::SetImageProxyUrl { url } => self.set_image_proxy_url(&url),
            AppAction::SetImageProxyKeyHex { key_hex } => self.set_image_proxy_key_hex(&key_hex),
            AppAction::SetImageProxySaltHex { salt_hex } => {
                self.set_image_proxy_salt_hex(&salt_hex)
            }
            AppAction::ResetImageProxySettings => self.reset_image_proxy_settings(),
            AppAction::SetMobilePushServerUrl { url } => self.set_mobile_push_server_url(&url),
            AppAction::ResetMobilePushServerUrl => self.reset_mobile_push_server_url(),
            AppAction::IngestMobilePushPayload { payload_json } => {
                self.ingest_mobile_push_payload(&payload_json)
            }
            AppAction::MarkMessagesSeen {
                chat_id,
                message_ids,
            } => self.mark_messages_seen(&chat_id, &message_ids),
            AppAction::SendReceipt {
                chat_id,
                receipt_type,
                message_ids,
            } => self.send_receipt(&chat_id, &receipt_type, message_ids),
            AppAction::DeleteLocalMessage {
                chat_id,
                message_id,
            } => self.delete_local_message(&chat_id, &message_id),
            AppAction::DeleteChat { chat_id } => self.delete_chat(&chat_id),
            AppAction::UpdateGroupName { group_id, name } => {
                self.update_group_name(&group_id, &name)
            }
            AppAction::UpdateGroupPicture {
                group_id,
                file_path,
                filename,
            } => self.update_group_picture(&group_id, &file_path, &filename),
            AppAction::UpdateGroupAbout { group_id, about } => {
                self.set_group_about(&group_id, about)
            }
            AppAction::AddGroupMembers {
                group_id,
                member_inputs,
            } => self.add_group_members(&group_id, &member_inputs),
            AppAction::SetGroupAdmin {
                group_id,
                owner_pubkey_hex,
                is_admin,
            } => self.set_group_admin(&group_id, &owner_pubkey_hex, is_admin),
            AppAction::RemoveGroupMember {
                group_id,
                owner_pubkey_hex,
            } => self.remove_group_member(&group_id, &owner_pubkey_hex),
            AppAction::UploadProfilePicture { file_path } => {
                self.upload_profile_picture(&file_path)
            }
            AppAction::AddAuthorizedDevice { device_input } => {
                self.add_authorized_device(&device_input)
            }
            AppAction::RemoveAuthorizedDevice { device_pubkey_hex } => {
                self.remove_authorized_device(&device_pubkey_hex)
            }
            AppAction::AcknowledgeRevokedDevice => self.acknowledge_revoked_device(),
            AppAction::PushScreen { screen } => self.push_screen(screen),
            AppAction::NavigateBack => self.navigate_back(),
            AppAction::UpdateScreenStack { stack } => self.update_screen_stack(stack),
            AppAction::SetChatDraft { chat_id, text } => self.set_chat_draft(&chat_id, &text),
        }
    }
}
