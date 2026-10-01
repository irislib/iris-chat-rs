use super::*;
use iris_chat_core::{ContactIdentitySnapshot, SocialBadge, SocialConnectionSnapshot};

#[derive(Subcommand)]
pub(super) enum ContactCommands {
    /// Show a chat contact's profile, saved name, and social connection.
    Show { contact: String },
    /// Add a private favorite star. Only you can see it.
    Favorite { contact: String },
    /// Remove a private favorite star.
    Unfavorite { contact: String },
    /// Approve the exact new name shown by `contact show`.
    ApproveName { contact: String, name: String },
    /// Follow this contact publicly using your Nostr follow list.
    Follow { contact: String },
    /// Remove this contact from your public Nostr follow list.
    Unfollow { contact: String },
}

pub(super) fn handle(cli: &CliApp, command: ContactCommands) -> Result<Value> {
    match command {
        ContactCommands::Show { contact } => Ok(contact_json(&direct_contact(cli, &contact)?)),
        ContactCommands::Favorite { contact } => set_favorite(cli, &contact, true),
        ContactCommands::Unfavorite { contact } => set_favorite(cli, &contact, false),
        ContactCommands::ApproveName { contact, name } => approve_name(cli, &contact, &name),
        ContactCommands::Follow { contact } => set_follow(cli, &contact, true),
        ContactCommands::Unfollow { contact } => set_follow(cli, &contact, false),
    }
}

fn direct_contact(cli: &CliApp, input: &str) -> Result<CurrentChatSnapshot> {
    let state = cli.app.state();
    require_account(&state)?;
    let id = if let Ok(id) = normalize_direct_chat_input(input) {
        id
    } else {
        let candidates: Vec<_> = state
            .chat_list
            .iter()
            .filter(|chat| {
                chat.chat_id == input
                    || chat.display_name.eq_ignore_ascii_case(input)
                    || chat.subtitle.as_deref() == Some(input)
            })
            .collect();
        anyhow::ensure!(
            candidates.len() <= 1,
            "More than one contact has that name. Use a user ID."
        );
        candidates
            .first()
            .context("Contact not found. Start a chat first.")?
            .chat_id
            .clone()
    };
    let thread = state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == id)
        .context("Contact not found. Start a chat first.")?;
    anyhow::ensure!(
        matches!(thread.kind, ChatKind::Direct),
        "Choose a direct chat contact."
    );
    // Route snapshots intentionally skip a contended database rather than block
    // a UI thread. A CLI command can briefly wait for that read-only projection.
    let started = Instant::now();
    loop {
        let chat = read_chat(cli, &id)?;
        if chat.contact_identity.is_some() {
            return Ok(chat);
        }
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(1),
            "Contact details are not available. Try again."
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn set_favorite(cli: &CliApp, input: &str, favorite: bool) -> Result<Value> {
    let chat = direct_contact(cli, input)?;
    if chat
        .contact_identity
        .as_ref()
        .is_some_and(|identity| identity.is_favorite != favorite)
    {
        let state = cli.dispatch_and_wait(
            AppAction::SetContactFavorite {
                owner_pubkey_hex: chat.chat_id.clone(),
                favorite,
            },
            Duration::from_secs(2),
        )?;
        fail_on_toast_except(&state, &["Public follow saved", "Public follow removed"])?;
    }
    let updated = direct_contact(cli, &chat.chat_id)?;
    anyhow::ensure!(
        updated
            .contact_identity
            .as_ref()
            .is_some_and(|identity| identity.is_favorite == favorite),
        "Could not save the favorite."
    );
    Ok(contact_json(&updated))
}

fn approve_name(cli: &CliApp, input: &str, expected: &str) -> Result<Value> {
    let chat = direct_contact(cli, input)?;
    let identity = chat.contact_identity.as_ref().unwrap();
    anyhow::ensure!(
        identity.pending_name.as_deref() == Some(expected),
        "That name is not pending. Review `iris contact show` and approve the exact new name."
    );
    let state = cli.dispatch_and_wait(
        AppAction::ApproveContactName {
            owner_pubkey_hex: chat.chat_id.clone(),
            name: expected.to_string(),
        },
        Duration::from_secs(2),
    )?;
    fail_on_toast_except(&state, &["Public follow saved", "Public follow removed"])?;
    let updated = direct_contact(cli, &chat.chat_id)?;
    anyhow::ensure!(
        updated
            .contact_identity
            .as_ref()
            .is_some_and(|identity| identity.saved_name.as_deref() == Some(expected)),
        "Name change was not confirmed. Review `iris contact show` before retrying."
    );
    Ok(contact_json(&updated))
}

fn set_follow(cli: &CliApp, input: &str, following: bool) -> Result<Value> {
    let chat = direct_contact(cli, input)?;
    let identity = chat.contact_identity.as_ref().unwrap();
    if identity.is_following == following {
        return Ok(contact_json(&chat));
    }
    anyhow::ensure!(
        identity.can_follow,
        "Public follows need your main device's secret key and a message server."
    );
    anyhow::ensure!(
        !identity.updating_follow,
        "A public follow change is already in progress."
    );
    // Wait for the action to clear any previous command's toast before polling.
    cli.dispatch_and_wait(
        AppAction::SetPublicFollow {
            owner_pubkey_hex: chat.chat_id.clone(),
            following,
        },
        Duration::from_secs(2),
    )?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(25) {
        let state = cli.app.state();
        fail_on_toast_except(&state, &["Public follow saved", "Public follow removed"])?;
        let updated = direct_contact(cli, &chat.chat_id)?;
        if updated
            .contact_identity
            .as_ref()
            .is_some_and(|identity| !identity.updating_follow && identity.is_following == following)
        {
            let mut result = contact_json(&updated);
            // The shared core queues publication; this is not a delivery receipt.
            result["network_publication"] = json!("not_verified");
            return Ok(result);
        }
        thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!(
        "Could not confirm the public follow update. Check `iris contact show` before retrying."
    )
}

fn contact_json(chat: &CurrentChatSnapshot) -> Value {
    json!({
        "user_id": chat.chat_id,
        "name": chat.display_name,
        "nickname": chat.nickname,
        "profile_name": chat.profile_name,
        "picture_url": chat.picture_url,
        "about": chat.about,
        "contact_identity": chat.contact_identity.as_ref().map(identity_json),
        "social_connection": chat.social_connection.as_ref().map(connection_json),
    })
}

pub(super) fn identity_json(identity: &ContactIdentitySnapshot) -> Value {
    json!({
        "is_following": identity.is_following,
        "can_follow": identity.can_follow,
        "updating_follow": identity.updating_follow,
        "first_seen_name": identity.first_seen_name,
        "saved_name": identity.saved_name,
        "pending_name": identity.pending_name,
        "is_favorite": identity.is_favorite,
    })
}

pub(super) fn connection_json(connection: &SocialConnectionSnapshot) -> Value {
    json!({
        "badge": connection.badge.as_ref().map(|badge| match badge {
            SocialBadge::Warning => "warning",
            SocialBadge::Following => "following",
            SocialBadge::Friend => "friend",
            SocialBadge::Trusted => "trusted",
            SocialBadge::Muted => "muted",
        }),
        "follow_distance": connection.follow_distance,
        "followed_by_friends": connection.followed_by_friends,
        "description": connection.description,
    })
}
