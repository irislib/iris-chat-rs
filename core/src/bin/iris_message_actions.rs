use super::*;

pub(super) fn handle_message_top_command(
    cli: &CliApp,
    command: MessageTopCommands,
) -> Result<Value> {
    match command {
        MessageTopCommands::Edit {
            chat,
            message_id,
            text,
        } => mutate_message(cli, &chat, &message_id, Some(&text), true),
        MessageTopCommands::Delete {
            chat,
            message_id,
            everyone,
        } => mutate_message(cli, &chat, &message_id, None, everyone),
        MessageTopCommands::EditHistory { chat, message_id } => {
            let chat_id = chat_action_input(&cli.app.state(), &chat);
            let current = read_chat(cli, &chat_id)?;
            let message = current
                .messages
                .iter()
                .find(|m| m.id == message_id)
                .context("Message not found.")?;
            Ok(
                json!({"id":message.id,"deleted_for_everyone":message.deleted_for_everyone,"edit_history":message.edit_history}),
            )
        }
        MessageTopCommands::Chat(command) => handle_chat_command(cli, command),
        MessageTopCommands::Send {
            chat,
            message,
            ttl,
            expires_at,
        } => {
            let expires_at = message_expiration(ttl, expires_at)?;
            send_message(cli, &chat, &message, expires_at)
        }
        MessageTopCommands::Read { chat, limit } => {
            read_chat(cli, &chat).map(|chat| chat_json(&chat, limit))
        }
        MessageTopCommands::Seen { chat, message_ids } => mark_seen(cli, &chat, message_ids),
        MessageTopCommands::React {
            chat,
            message_id,
            emoji,
        } => react(cli, &chat, &message_id, &emoji),
        MessageTopCommands::Typing { chat, stop } => typing(cli, &chat, stop),
        MessageTopCommands::Receipt {
            chat,
            receipt_type,
            message_ids,
        } => receipt(cli, &chat, &receipt_type, message_ids),
        MessageTopCommands::Search { .. }
        | MessageTopCommands::Tail { .. }
        | MessageTopCommands::Listen { .. } => {
            unreachable!("streaming and read-only commands are handled before regular dispatch")
        }
    }
}

fn mutate_message(
    cli: &CliApp,
    chat: &str,
    message_id: &str,
    text: Option<&str>,
    everyone: bool,
) -> Result<Value> {
    let chat_id = chat_action_input(&cli.app.state(), chat);
    let current = read_chat(cli, &chat_id)?;
    let message = current
        .messages
        .iter()
        .find(|m| m.id == message_id)
        .context("Message not found.")?;
    if everyone && (!message.is_outgoing || message.deleted_for_everyone) {
        anyhow::bail!("Only your own messages can be changed for everyone.");
    }
    let action = if let Some(text) = text {
        AppAction::EditMessage {
            chat_id: chat_id.clone(),
            message_id: message_id.into(),
            text: text.into(),
        }
    } else if everyone {
        AppAction::DeleteMessageForEveryone {
            chat_id: chat_id.clone(),
            message_id: message_id.into(),
        }
    } else {
        AppAction::DeleteLocalMessage {
            chat_id: chat_id.clone(),
            message_id: message_id.into(),
        }
    };
    let state = cli.dispatch_and_wait(action, Duration::from_secs(8))?;
    fail_on_toast(&state)?;
    let current = read_chat(cli, &chat_id)?;
    if !everyone && text.is_none() {
        return Ok(
            json!({"id":message_id,"deleted_locally":!current.messages.iter().any(|m|m.id==message_id)}),
        );
    }
    let updated = current
        .messages
        .iter()
        .find(|m| m.id == message_id)
        .context("Message not found.")?;
    if text.is_some_and(|text| updated.body != text.trim())
        || (text.is_none() && !updated.deleted_for_everyone)
    {
        anyhow::bail!("Message could not be changed.");
    }
    wait_after_send_network_idle(cli)?;
    Ok(message_json(updated))
}
