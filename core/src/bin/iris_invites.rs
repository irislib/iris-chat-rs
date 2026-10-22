fn handle_invite_device_command(cli: &CliApp, command: InviteDeviceTopCommands) -> Result<Value> {
    match command {
        InviteDeviceTopCommands::Invite(InviteCommands::Create { full }) => {
            cli.dispatch_and_wait(
                if full {
                    AppAction::CreateFullInvite
                } else {
                    AppAction::CreatePublicInvite
                },
                Duration::from_secs(40),
            )?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            let invite = state.public_invite.context("No invite was created.")?;
            Ok(json!({ "url": invite.url }))
        }
        InviteDeviceTopCommands::Invite(InviteCommands::Group {
            group,
            max_uses,
            expires_in,
        }) => {
            let group_id = group.strip_prefix("group:").unwrap_or(&group).to_string();
            cli.dispatch_and_wait(
                AppAction::PushScreen {
                    screen: iris_chat_core::Screen::GroupDetails {
                        group_id: group_id.clone(),
                    },
                },
                Duration::from_secs(3),
            )?;
            cli.dispatch_and_wait(
                AppAction::CreateGroupInvite {
                    group_id,
                    max_uses,
                    expires_in_seconds: (expires_in > 0).then_some(expires_in),
                },
                Duration::from_secs(40),
            )?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            let links = state.group_details.context("Group not found")?.invite_links;
            Ok(
                json!({"invites": links.iter().map(|i| json!({"id": i.id, "url": i.url, "uses": i.use_count, "max_uses": i.max_uses, "expires_at": i.expires_at})).collect::<Vec<_>>() }),
            )
        }
        InviteDeviceTopCommands::Invite(InviteCommands::Revoke { group, invite_id }) => {
            cli.dispatch_and_wait(
                AppAction::RevokeGroupInvite {
                    group_id: group.trim_start_matches("group:").to_string(),
                    invite_id,
                },
                Duration::from_secs(3),
            )?;
            fail_on_toast(&cli.app.state())?;
            Ok(json!({"revoked": true}))
        }
        InviteDeviceTopCommands::Invite(InviteCommands::Accept { invite }) => {
            cli.dispatch_and_wait(
                AppAction::AcceptInvite {
                    invite_input: invite,
                },
                Duration::from_secs(30),
            )?;
            let state = cli.app.state();
            let pending_group = matches!(
                state.toast.as_deref(),
                Some("Waiting for the group admin to connect…" | "Joining group…")
            );
            if !pending_group {
                fail_on_toast(&state)?;
            }
            Ok(json!({
                "pending_group_join": pending_group,
                "chats": chat_list_json(&state),
                "current_chat": state.current_chat.as_ref().map(chat_summary_json),
            }))
        }
        InviteDeviceTopCommands::Link(LinkCommands::Create) => {
            cli.dispatch_and_wait(
                AppAction::StartLinkedDevice {
                    owner_input: String::new(),
                },
                Duration::from_secs(3),
            )?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            let link = state.link_device.context("No link code was created.")?;
            Ok(json!({
                "url": link.url,
                "device_input": link.device_input,
            }))
        }
        InviteDeviceTopCommands::Link(LinkCommands::Accept {
            invite,
            include_message_history,
        }) => {
            let is_signer_link = invite.trim().starts_with("nostrconnect://");
            let previous_count = cli
                .app
                .state()
                .device_roster
                .map_or(0, |roster| roster.devices.len());
            cli.dispatch_and_wait(
                if include_message_history {
                    AppAction::AddAuthorizedDeviceWithHistory {
                        device_input: invite,
                        include_message_history: true,
                    }
                } else {
                    AppAction::AddAuthorizedDevice {
                        device_input: invite,
                    }
                },
                Duration::from_secs(4),
            )?;
            if is_signer_link {
                let deadline = Instant::now() + Duration::from_secs(180);
                loop {
                    let state = cli.app.state();
                    fail_on_toast_except(&state, &["Device added"])?;
                    if state
                        .device_roster
                        .is_some_and(|roster| roster.devices.len() > previous_count)
                    {
                        break;
                    }
                    anyhow::ensure!(
                        Instant::now() < deadline,
                        "Device linking timed out. Try again."
                    );
                    thread::sleep(Duration::from_millis(50));
                }
            }
            let state = cli.app.state();
            fail_on_toast_except(&state, &["Device added"])?;
            Ok(json!({
                "accepted": true,
                "device_roster": state.device_roster.map(|roster| {
                    json!({
                        "device_count": roster.devices.len(),
                        "devices": roster.devices.iter().map(|device| {
                            json!({
                                "device_id": device.device_pubkey_hex,
                                "device_npub": device.device_npub,
                                "current": device.is_current_device,
                                "authorized": device.is_authorized,
                                "stale": device.is_stale,
                            })
                        }).collect::<Vec<_>>(),
                    })
                }),
            }))
        }
    }
}
