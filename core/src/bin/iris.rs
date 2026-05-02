use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iris_chat_core::{
    AppAction, AppReconciler, AppState, AppUpdate, ChatKind, ChatMessageSnapshot,
    ChatThreadSnapshot, CurrentChatSnapshot, DeliveryState, DesktopNearbyObserver,
    DesktopNearbySnapshot, DeviceAuthorizationState, FfiApp, FfiDesktopNearby,
    GroupDetailsSnapshot,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "iris")]
#[command(version)]
#[command(about = "Iris Chat command line client")]
struct Cli {
    #[arg(short, long, global = true)]
    json: bool,

    #[arg(long, global = true, env = "IRIS_DATA_DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Login {
        secret_key: String,
    },
    Restore {
        secret_key: String,
    },
    Logout,
    Whoami,
    State,
    Sync {
        #[arg(long, default_value_t = 1500)]
        wait_ms: u64,
    },
    Search {
        query: String,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    Tail {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
        #[arg(short, long)]
        follow: bool,
        #[arg(short, long)]
        chat: Option<String>,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },
    Listen {
        #[arg(short, long)]
        chat: Option<String>,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        nearby_lan: bool,
    },
    #[command(subcommand)]
    Account(AccountCommands),
    #[command(subcommand)]
    Chat(ChatCommands),
    Send {
        chat: String,
        message: String,
        #[arg(long)]
        ttl: Option<u64>,
        #[arg(long, value_name = "UNIX_SECONDS")]
        expires_at: Option<u64>,
    },
    React {
        chat: String,
        message_id: String,
        emoji: String,
    },
    Typing {
        chat: String,
        #[arg(long)]
        stop: bool,
    },
    Receipt {
        chat: String,
        receipt_type: String,
        message_ids: Vec<String>,
    },
    Read {
        chat: String,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    Seen {
        chat: String,
        message_ids: Vec<String>,
    },
    #[command(subcommand)]
    Invite(InviteCommands),
    #[command(subcommand)]
    Link(LinkCommands),
    #[command(subcommand)]
    Group(GroupCommands),
    #[command(subcommand)]
    Relay(RelayCommands),
}

#[derive(Subcommand)]
enum AccountCommands {
    Create {
        #[arg(short, long, default_value = "")]
        name: String,
    },
    Restore {
        secret_key: String,
    },
    Bundle,
}

#[derive(Subcommand)]
enum ChatCommands {
    List,
    Create {
        user_id: String,
    },
    Open {
        chat: String,
    },
    Read {
        chat: String,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    Send {
        chat: String,
        message: String,
        #[arg(long)]
        ttl: Option<u64>,
        #[arg(long, value_name = "UNIX_SECONDS")]
        expires_at: Option<u64>,
    },
    Seen {
        chat: String,
        message_ids: Vec<String>,
    },
    Delete {
        chat: String,
    },
    Ttl {
        chat: String,
        seconds: Option<u64>,
    },
    Mute {
        chat: String,
        muted: bool,
    },
}

#[derive(Subcommand)]
enum InviteCommands {
    Create,
    Accept { invite: String },
}

#[derive(Subcommand)]
enum LinkCommands {
    Create,
    Accept { invite: String },
}

#[derive(Subcommand)]
enum GroupCommands {
    Create {
        name: String,
        members: Vec<String>,
    },
    List,
    Show {
        group: String,
    },
    Send {
        group: String,
        message: String,
    },
    Read {
        group: String,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    Add {
        group: String,
        members: Vec<String>,
    },
    Remove {
        group: String,
        members: Vec<String>,
    },
    AddAdmin {
        group: String,
        member: String,
    },
    RemoveAdmin {
        group: String,
        member: String,
    },
    Rename {
        group: String,
        name: String,
    },
    React {
        group: String,
        message_id: String,
        emoji: String,
    },
    Delete {
        group: String,
    },
}

#[derive(Subcommand)]
enum RelayCommands {
    List,
    Add { url: String },
    Remove { url: String },
    Set { urls: Vec<String> },
    Reset,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
struct AccountBundle {
    owner_nsec: Option<String>,
    owner_pubkey_hex: String,
    device_nsec: String,
}

#[derive(Default)]
struct CliReconciler {
    data_dir: PathBuf,
    updates: Mutex<Vec<AppUpdate>>,
}

struct ReconcilerHandle(Arc<CliReconciler>);

impl AppReconciler for ReconcilerHandle {
    fn reconcile(&self, update: AppUpdate) {
        if let AppUpdate::PersistAccountBundle {
            owner_nsec,
            owner_pubkey_hex,
            device_nsec,
            ..
        } = &update
        {
            let bundle = AccountBundle {
                owner_nsec: owner_nsec.clone(),
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                device_nsec: device_nsec.clone(),
            };
            let _ = write_account_bundle(&self.0.data_dir, &bundle);
        }
        if let Ok(mut updates) = self.0.updates.lock() {
            updates.push(update);
        }
    }
}

struct CliApp {
    app: Arc<FfiApp>,
    reconciler: Arc<CliReconciler>,
}

struct CliNearbyObserver;

impl DesktopNearbyObserver for CliNearbyObserver {
    fn desktop_nearby_changed(&self, _snapshot: DesktopNearbySnapshot) {}
}

#[derive(Serialize)]
struct Envelope<T: Serialize> {
    status: &'static str,
    command: String,
    data: T,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    status: &'static str,
    error: String,
}

fn main() {
    let cli = Cli::parse();
    let json_output = cli.json;
    if let Err(error) = run(cli) {
        if json_output {
            println!(
                "{}",
                serde_json::to_string(&ErrorEnvelope {
                    status: "error",
                    error: error.to_string(),
                })
                .unwrap()
            );
        } else {
            eprintln!("{}", error);
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("create data dir {}", data_dir.display()))?;
    let command_name = command_name(&cli.command).to_string();
    let data = match cli.command {
        Commands::Search { query, limit } => search_messages(&data_dir, &query, limit)?,
        Commands::Tail {
            limit,
            follow,
            chat,
            interval_ms,
        } => {
            if follow {
                follow_messages(&data_dir, chat.as_deref(), interval_ms, "tail")?;
                return Ok(());
            }
            tail_messages(&data_dir, limit, chat.as_deref())?
        }
        Commands::Listen {
            chat,
            interval_ms,
            nearby_lan,
        } => {
            listen(&data_dir, chat.as_deref(), interval_ms, nearby_lan)?;
            return Ok(());
        }
        command => {
            let cli_app = CliApp::open(&data_dir)?;
            let data = handle_command(&cli_app, &data_dir, command)?;
            cli_app.app.shutdown();
            data
        }
    };
    print_output(cli.json, &command_name, data)
}

impl CliApp {
    fn open(data_dir: &Path) -> Result<Self> {
        let app = FfiApp::new(
            data_dir.to_string_lossy().to_string(),
            String::new(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        let reconciler = Arc::new(CliReconciler {
            data_dir: data_dir.to_path_buf(),
            updates: Mutex::new(Vec::new()),
        });
        app.listen_for_updates(Box::new(ReconcilerHandle(reconciler.clone())));
        fail_on_toast(&app.state())?;
        let cli_app = Self { app, reconciler };
        if let Some(bundle) = read_account_bundle(data_dir)? {
            cli_app.dispatch_and_wait(
                AppAction::RestoreAccountBundle {
                    owner_nsec: bundle.owner_nsec,
                    owner_pubkey_hex: bundle.owner_pubkey_hex,
                    device_nsec: bundle.device_nsec,
                },
                Duration::from_secs(3),
            )?;
        }
        Ok(cli_app)
    }

    fn dispatch_and_wait(&self, action: AppAction, timeout: Duration) -> Result<AppState> {
        let before = self.app.state().rev;
        self.app.dispatch(action);
        self.wait_for_quiet_after(before, timeout, false)
    }

    fn dispatch_and_wait_network(&self, action: AppAction, timeout: Duration) -> Result<AppState> {
        let before = self.app.state().rev;
        self.app.dispatch(action);
        self.wait_for_quiet_after(before, timeout, true)
    }

    fn wait_for_quiet_after(
        &self,
        before_rev: u64,
        timeout: Duration,
        include_network_sync: bool,
    ) -> Result<AppState> {
        let started = Instant::now();
        let mut saw_change = false;
        let mut last_rev = before_rev;
        let mut stable_since = Instant::now();
        while started.elapsed() < timeout {
            let state = self.app.state();
            if state.rev != last_rev {
                saw_change = state.rev != before_rev || saw_change;
                last_rev = state.rev;
                stable_since = Instant::now();
            }
            let busy = if include_network_sync {
                is_busy_or_syncing(&state)
            } else {
                is_busy(&state)
            };
            if saw_change && !busy && stable_since.elapsed() >= Duration::from_millis(80) {
                return Ok(state);
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(self.app.state())
    }

    fn wait_for_update_count(&self, before_count: usize, timeout: Duration) {
        let started = Instant::now();
        while started.elapsed() < timeout {
            let count = self
                .reconciler
                .updates
                .lock()
                .map(|updates| updates.len())
                .unwrap_or(before_count);
            if count > before_count {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

fn handle_command(cli: &CliApp, data_dir: &Path, command: Commands) -> Result<Value> {
    match command {
        Commands::Login { secret_key } | Commands::Restore { secret_key } => {
            restore_account(cli, &secret_key)
        }
        Commands::Logout => {
            cli.dispatch_and_wait(AppAction::Logout, Duration::from_secs(2))?;
            let _ = std::fs::remove_file(account_bundle_path(data_dir));
            Ok(json!({ "logged_out": true }))
        }
        Commands::Whoami => Ok(account_json(&require_account(&cli.app.state())?)),
        Commands::State => Ok(state_json(&cli.app.state())),
        Commands::Sync { wait_ms } => {
            let state = cli.dispatch_and_wait_network(
                AppAction::AppForegrounded,
                Duration::from_millis(wait_ms.max(100)),
            )?;
            Ok(state_json(&state))
        }
        Commands::Search { .. } | Commands::Tail { .. } | Commands::Listen { .. } => {
            unreachable!("streaming and read-only commands are handled before regular dispatch")
        }
        Commands::Account(AccountCommands::Create { name }) => {
            cli.dispatch_and_wait(AppAction::CreateAccount { name }, Duration::from_secs(4))?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            Ok(account_json(&require_account(&state)?))
        }
        Commands::Account(AccountCommands::Restore { secret_key }) => {
            restore_account(cli, &secret_key)
        }
        Commands::Account(AccountCommands::Bundle) => {
            let bundle = read_account_bundle(data_dir)?.context("No saved account bundle.")?;
            Ok(json!({
                "owner_pubkey_hex": bundle.owner_pubkey_hex,
                "has_owner_secret": bundle.owner_nsec.is_some(),
                "has_device_secret": !bundle.device_nsec.is_empty(),
            }))
        }
        Commands::Chat(ChatCommands::List) => {
            Ok(json!({ "chats": chat_list_json(&cli.app.state()) }))
        }
        Commands::Chat(ChatCommands::Create { user_id }) => create_chat(cli, &user_id),
        Commands::Chat(ChatCommands::Open { chat }) => {
            open_chat(cli, &chat).map(|chat| chat_json(&chat, usize::MAX))
        }
        Commands::Chat(ChatCommands::Read { chat, limit }) | Commands::Read { chat, limit } => {
            open_chat(cli, &chat).map(|chat| chat_json(&chat, limit))
        }
        Commands::Chat(ChatCommands::Send {
            chat,
            message,
            ttl,
            expires_at,
        })
        | Commands::Send {
            chat,
            message,
            ttl,
            expires_at,
        } => {
            let expires_at = message_expiration(ttl, expires_at)?;
            send_message(cli, &chat, &message, expires_at)
        }
        Commands::React {
            chat,
            message_id,
            emoji,
        } => react(cli, &chat, &message_id, &emoji),
        Commands::Typing { chat, stop } => typing(cli, &chat, stop),
        Commands::Receipt {
            chat,
            receipt_type,
            message_ids,
        } => receipt(cli, &chat, &receipt_type, message_ids),
        Commands::Chat(ChatCommands::Seen { chat, message_ids })
        | Commands::Seen { chat, message_ids } => mark_seen(cli, &chat, message_ids),
        Commands::Chat(ChatCommands::Delete { chat }) => {
            let chat_id = resolve_chat_id(&cli.app.state(), &chat)?;
            cli.dispatch_and_wait(
                AppAction::DeleteChat {
                    chat_id: chat_id.clone(),
                },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "chat_id": chat_id, "deleted": true }))
        }
        Commands::Chat(ChatCommands::Ttl { chat, seconds }) => {
            let chat_id = resolve_chat_id(&cli.app.state(), &chat)?;
            cli.dispatch_and_wait(
                AppAction::SetChatMessageTtl {
                    chat_id: chat_id.clone(),
                    ttl_seconds: seconds,
                },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "chat_id": chat_id, "message_ttl_seconds": seconds }))
        }
        Commands::Chat(ChatCommands::Mute { chat, muted }) => {
            let chat_id = resolve_chat_id(&cli.app.state(), &chat)?;
            cli.dispatch_and_wait(
                AppAction::SetChatMuted {
                    chat_id: chat_id.clone(),
                    muted,
                },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "chat_id": chat_id, "muted": muted }))
        }
        Commands::Invite(InviteCommands::Create) => {
            cli.dispatch_and_wait(AppAction::CreatePublicInvite, Duration::from_secs(3))?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            let invite = state.public_invite.context("No invite was created.")?;
            Ok(json!({ "url": invite.url }))
        }
        Commands::Invite(InviteCommands::Accept { invite }) => {
            cli.dispatch_and_wait(
                AppAction::AcceptInvite {
                    invite_input: invite,
                },
                Duration::from_secs(4),
            )?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            Ok(json!({
                "chats": chat_list_json(&state),
                "current_chat": state.current_chat.as_ref().map(|chat| chat_summary_json(chat)),
            }))
        }
        Commands::Link(LinkCommands::Create) => {
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
        Commands::Link(LinkCommands::Accept { invite }) => {
            cli.dispatch_and_wait(
                AppAction::AddAuthorizedDevice {
                    device_input: invite,
                },
                Duration::from_secs(4),
            )?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
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
        Commands::Group(GroupCommands::Create { name, members }) => {
            cli.dispatch_and_wait(
                AppAction::CreateGroup {
                    name,
                    member_inputs: members,
                },
                Duration::from_secs(4),
            )?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            Ok(json!({
                "groups": group_list_json(&state),
                "current_chat": state.current_chat.as_ref().map(|chat| chat_summary_json(chat)),
            }))
        }
        Commands::Group(GroupCommands::List) => {
            Ok(json!({ "groups": group_list_json(&cli.app.state()) }))
        }
        Commands::Group(GroupCommands::Show { group }) => show_group(cli, &group),
        Commands::Group(GroupCommands::Send { group, message }) => {
            send_message(cli, &normalize_group_chat(&group), &message, None)
        }
        Commands::Group(GroupCommands::Read { group, limit }) => {
            open_chat(cli, &normalize_group_chat(&group)).map(|chat| chat_json(&chat, limit))
        }
        Commands::Group(GroupCommands::Add { group, members }) => {
            let group_id = resolve_group_id(&cli.app.state(), &group)?;
            cli.dispatch_and_wait(
                AppAction::AddGroupMembers {
                    group_id: group_id.clone(),
                    member_inputs: members,
                },
                Duration::from_secs(3),
            )?;
            show_group(cli, &group_id)
        }
        Commands::Group(GroupCommands::Remove { group, members }) => {
            let group_id = resolve_group_id(&cli.app.state(), &group)?;
            if members.is_empty() {
                anyhow::bail!("At least one member is required.");
            }
            for member in members {
                let owner_pubkey_hex = owner_input_to_hex(&member)?;
                cli.dispatch_and_wait(
                    AppAction::RemoveGroupMember {
                        group_id: group_id.clone(),
                        owner_pubkey_hex,
                    },
                    Duration::from_secs(3),
                )?;
            }
            show_group(cli, &group_id)
        }
        Commands::Group(GroupCommands::AddAdmin { group, member }) => {
            set_group_admin(cli, &group, &member, true)
        }
        Commands::Group(GroupCommands::RemoveAdmin { group, member }) => {
            set_group_admin(cli, &group, &member, false)
        }
        Commands::Group(GroupCommands::Rename { group, name }) => {
            let group_id = resolve_group_id(&cli.app.state(), &group)?;
            cli.dispatch_and_wait(
                AppAction::UpdateGroupName {
                    group_id: group_id.clone(),
                    name,
                },
                Duration::from_secs(3),
            )?;
            show_group(cli, &group_id)
        }
        Commands::Group(GroupCommands::React {
            group,
            message_id,
            emoji,
        }) => react(cli, &normalize_group_chat(&group), &message_id, &emoji),
        Commands::Group(GroupCommands::Delete { group }) => {
            let group_id = resolve_group_id(&cli.app.state(), &group)?;
            let chat_id = normalize_group_chat(&group_id);
            cli.dispatch_and_wait(
                AppAction::DeleteChat {
                    chat_id: chat_id.clone(),
                },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "chat_id": chat_id, "group_id": group_id, "deleted": true }))
        }
        Commands::Relay(RelayCommands::List) => {
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        Commands::Relay(RelayCommands::Add { url }) => {
            cli.dispatch_and_wait(
                AppAction::AddNostrRelay { relay_url: url },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        Commands::Relay(RelayCommands::Remove { url }) => {
            cli.dispatch_and_wait(
                AppAction::RemoveNostrRelay { relay_url: url },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        Commands::Relay(RelayCommands::Reset) => {
            cli.dispatch_and_wait(AppAction::ResetNostrRelays, Duration::from_secs(2))?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        Commands::Relay(RelayCommands::Set { urls }) => {
            cli.dispatch_and_wait(
                AppAction::SetNostrRelays { relay_urls: urls },
                Duration::from_secs(3),
            )?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
    }
}

fn restore_account(cli: &CliApp, secret_key: &str) -> Result<Value> {
    let before = cli
        .reconciler
        .updates
        .lock()
        .map(|updates| updates.len())
        .unwrap_or(0);
    cli.dispatch_and_wait(
        AppAction::RestoreSession {
            owner_nsec: secret_key.to_string(),
        },
        Duration::from_secs(4),
    )?;
    cli.wait_for_update_count(before, Duration::from_secs(2));
    let state = cli.app.state();
    fail_on_toast(&state)?;
    Ok(account_json(&require_account(&state)?))
}

fn create_chat(cli: &CliApp, user_id: &str) -> Result<Value> {
    cli.dispatch_and_wait(
        AppAction::CreateChat {
            peer_input: user_id.to_string(),
        },
        Duration::from_secs(3),
    )?;
    let state = cli.app.state();
    fail_on_toast(&state)?;
    let chat = state.current_chat.context("No chat was opened.")?;
    Ok(chat_json(&chat, usize::MAX))
}

fn open_chat(cli: &CliApp, chat: &str) -> Result<CurrentChatSnapshot> {
    let chat_id = chat_action_input(&cli.app.state(), chat);
    cli.dispatch_and_wait(AppAction::OpenChat { chat_id }, Duration::from_secs(2))?;
    let state = cli.app.state();
    fail_on_toast(&state)?;
    state.current_chat.context("No chat is open.")
}

fn send_message(
    cli: &CliApp,
    chat: &str,
    message: &str,
    expires_at_secs: Option<u64>,
) -> Result<Value> {
    let chat_id = chat_action_input(&cli.app.state(), chat);
    let action = if let Some(expires_at_secs) = expires_at_secs {
        AppAction::SendDisappearingMessage {
            chat_id: chat_id.clone(),
            text: message.to_string(),
            expires_at_secs,
        }
    } else {
        AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text: message.to_string(),
        }
    };
    cli.dispatch_and_wait(action, Duration::from_secs(3))?;
    let state = cli.app.state();
    fail_on_toast(&state)?;
    let current = state.current_chat.context("No chat is open.")?;
    let sent = current
        .messages
        .iter()
        .rev()
        .find(|item| item.is_outgoing && item.body == message)
        .cloned()
        .context("Message was not added to the chat.")?;
    Ok(message_json(&sent))
}

fn react(cli: &CliApp, chat: &str, message_id: &str, emoji: &str) -> Result<Value> {
    let chat_id = chat_action_input(&cli.app.state(), chat);
    cli.dispatch_and_wait(
        AppAction::ToggleReaction {
            chat_id: chat_id.clone(),
            message_id: message_id.to_string(),
            emoji: emoji.to_string(),
        },
        Duration::from_secs(2),
    )?;
    let current = open_chat(cli, &chat_id)?;
    let message = current
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .context("Message not found.")?;
    Ok(message_json(message))
}

fn typing(cli: &CliApp, chat: &str, stop: bool) -> Result<Value> {
    let chat_id = chat_action_input(&cli.app.state(), chat);
    cli.dispatch_and_wait(
        AppAction::SetTypingIndicatorsEnabled { enabled: true },
        Duration::from_secs(1),
    )?;
    let action = if stop {
        AppAction::StopTyping {
            chat_id: chat_id.clone(),
        }
    } else {
        AppAction::SendTyping {
            chat_id: chat_id.clone(),
        }
    };
    cli.dispatch_and_wait(action, Duration::from_secs(2))?;
    Ok(json!({ "chat_id": chat_id, "typing": !stop }))
}

fn receipt(
    cli: &CliApp,
    chat: &str,
    receipt_type: &str,
    message_ids: Vec<String>,
) -> Result<Value> {
    let receipt_type = receipt_type.trim().to_ascii_lowercase();
    if receipt_type != "delivered" && receipt_type != "seen" {
        anyhow::bail!("Receipt type must be delivered or seen.");
    }
    if receipt_type == "seen" {
        return mark_seen(cli, chat, message_ids);
    }
    let chat_id = chat_action_input(&cli.app.state(), chat);
    cli.dispatch_and_wait(
        AppAction::SendReceipt {
            chat_id: chat_id.clone(),
            receipt_type: receipt_type.clone(),
            message_ids: message_ids.clone(),
        },
        Duration::from_secs(2),
    )?;
    Ok(json!({
        "chat_id": chat_id,
        "receipt_type": receipt_type,
        "message_ids": message_ids,
    }))
}

fn mark_seen(cli: &CliApp, chat: &str, message_ids: Vec<String>) -> Result<Value> {
    let current = open_chat(cli, chat)?;
    let ids = if message_ids.is_empty() {
        current
            .messages
            .iter()
            .filter(|message| !message.is_outgoing)
            .map(|message| message.id.clone())
            .collect()
    } else {
        message_ids
    };
    cli.dispatch_and_wait(
        AppAction::MarkMessagesSeen {
            chat_id: current.chat_id.clone(),
            message_ids: ids.clone(),
        },
        Duration::from_secs(2),
    )?;
    Ok(json!({ "chat_id": current.chat_id, "message_ids": ids }))
}

fn message_expiration(ttl: Option<u64>, expires_at: Option<u64>) -> Result<Option<u64>> {
    match (ttl, expires_at) {
        (Some(_), Some(_)) => anyhow::bail!("Use either --ttl or --expires-at, not both."),
        (Some(ttl), None) if ttl > 0 => Ok(Some(now_secs()?.saturating_add(ttl))),
        (Some(_), None) => Ok(None),
        (None, Some(expires_at)) if expires_at > 0 => Ok(Some(expires_at)),
        (None, Some(_)) | (None, None) => Ok(None),
    }
}

fn now_secs() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn show_group(cli: &CliApp, group: &str) -> Result<Value> {
    let group_id = resolve_group_id(&cli.app.state(), group)?;
    cli.dispatch_and_wait(
        AppAction::PushScreen {
            screen: iris_chat_core::Screen::GroupDetails {
                group_id: group_id.clone(),
            },
        },
        Duration::from_secs(2),
    )?;
    let state = cli.app.state();
    let details = state.group_details.context("No group details available.")?;
    Ok(group_json(&details))
}

fn set_group_admin(cli: &CliApp, group: &str, member: &str, is_admin: bool) -> Result<Value> {
    let group_id = resolve_group_id(&cli.app.state(), group)?;
    let owner_pubkey_hex = owner_input_to_hex(member)?;
    cli.dispatch_and_wait(
        AppAction::SetGroupAdmin {
            group_id: group_id.clone(),
            owner_pubkey_hex,
            is_admin,
        },
        Duration::from_secs(3),
    )?;
    show_group(cli, &group_id)
}

fn owner_input_to_hex(input: &str) -> Result<String> {
    let hex = iris_chat_core::peer_input_to_hex(input.to_string());
    if hex.is_empty() {
        anyhow::bail!("Invalid user ID: {input}");
    }
    Ok(hex)
}

fn search_messages(data_dir: &Path, query: &str, limit: usize) -> Result<Value> {
    let conn = open_existing_db(data_dir)?;
    let pattern = format!("%{query}%");
    let mut stmt = conn.prepare(
        "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
         FROM messages
         WHERE body LIKE ?1
         ORDER BY created_at_secs DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map((&pattern, limit as i64), |row| {
        Ok(json!({
            "chat_id": row.get::<_, String>(0)?,
            "id": row.get::<_, String>(1)?,
            "body": row.get::<_, String>(2)?,
            "is_outgoing": row.get::<_, i64>(3)? != 0,
            "created_at_secs": row.get::<_, i64>(4)?,
            "delivery": row.get::<_, String>(5)?,
        }))
    })?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    Ok(json!({ "messages": messages }))
}

fn tail_messages(data_dir: &Path, limit: usize, chat: Option<&str>) -> Result<Value> {
    let conn = open_existing_db(data_dir)?;
    let sql = match chat {
        Some(_) => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             WHERE chat_id = ?1
             ORDER BY created_at_secs DESC, id DESC
             LIMIT ?2"
        }
        None => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             ORDER BY created_at_secs DESC, id DESC
             LIMIT ?1"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Value> {
        Ok(json!({
            "chat_id": row.get::<_, String>(0)?,
            "id": row.get::<_, String>(1)?,
            "body": row.get::<_, String>(2)?,
            "is_outgoing": row.get::<_, i64>(3)? != 0,
            "created_at_secs": row.get::<_, i64>(4)?,
            "delivery": row.get::<_, String>(5)?,
        }))
    };
    let mut messages = Vec::new();
    match chat {
        Some(chat_id) => {
            let rows = stmt.query_map((chat_id, limit as i64), map_row)?;
            for row in rows {
                messages.push(row?);
            }
        }
        None => {
            let rows = stmt.query_map([limit as i64], map_row)?;
            for row in rows {
                messages.push(row?);
            }
        }
    }
    messages.reverse();
    Ok(json!({ "messages": messages }))
}

fn listen(data_dir: &Path, chat: Option<&str>, interval_ms: u64, nearby_lan: bool) -> Result<()> {
    let interval = Duration::from_millis(interval_ms.max(100));
    let cli = CliApp::open(data_dir)?;
    let state = cli.app.state();
    fail_on_toast(&state)?;
    require_account(&state)?;
    let chat_filter = normalize_chat_filter(&state, chat);
    if let Some(chat_id) = chat_filter.as_ref() {
        cli.dispatch_and_wait(
            AppAction::OpenChat {
                chat_id: chat_id.clone(),
            },
            Duration::from_secs(2),
        )?;
        fail_on_toast(&cli.app.state())?;
    }
    let mut seen = latest_message_keys(data_dir, chat_filter.as_deref())?;
    let _nearby = if nearby_lan {
        let service = FfiDesktopNearby::new(cli.app.clone(), Box::new(CliNearbyObserver));
        let name = state
            .account
            .as_ref()
            .map(|account| account.display_name.trim())
            .filter(|name| !name.is_empty())
            .unwrap_or("Iris")
            .to_string();
        service.start(name);
        Some(service)
    } else {
        None
    };
    let state = cli.dispatch_and_wait(AppAction::AppForegrounded, Duration::from_secs(5))?;
    fail_on_toast(&state)?;

    print_stream_envelope(
        "listen",
        json!({
            "ready": true,
            "chat": chat_filter.clone(),
            "network": true,
            "nearby_lan": nearby_lan,
        }),
    )?;
    let mut last_foreground = Instant::now();

    loop {
        thread::sleep(interval);
        if last_foreground.elapsed() >= Duration::from_secs(60) {
            cli.app.dispatch(AppAction::AppForegrounded);
            last_foreground = Instant::now();
        }
        stream_new_messages(data_dir, chat_filter.as_deref(), &mut seen)?;
    }
}

fn follow_messages(
    data_dir: &Path,
    chat: Option<&str>,
    interval_ms: u64,
    command: &str,
) -> Result<()> {
    let interval = Duration::from_millis(interval_ms.max(100));
    let mut seen = latest_message_keys(data_dir, chat)?;
    print_stream_envelope(
        command,
        json!({ "ready": true, "chat": chat, "network": false }),
    )?;
    loop {
        thread::sleep(interval);
        stream_new_messages(data_dir, chat, &mut seen)?;
    }
}

fn stream_new_messages(
    data_dir: &Path,
    chat: Option<&str>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<()> {
    let messages = new_message_rows(data_dir, chat, seen)?;
    for message in messages {
        if let (Some(chat_id), Some(id)) = (
            message.get("chat_id").and_then(Value::as_str),
            message.get("id").and_then(Value::as_str),
        ) {
            seen.insert(format!("{chat_id}\0{id}"));
        }
        print_stream_envelope("message", message)?;
    }
    Ok(())
}

fn print_stream_envelope(command: &str, data: Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&Envelope {
            status: "ok",
            command: command.to_string(),
            data,
        })?
    );
    std::io::stdout().flush()?;
    Ok(())
}

fn normalize_chat_filter(state: &AppState, chat: Option<&str>) -> Option<String> {
    chat.map(|input| chat_action_input(state, input))
}

fn latest_message_keys(
    data_dir: &Path,
    chat: Option<&str>,
) -> Result<std::collections::HashSet<String>> {
    let conn = open_existing_db(data_dir)?;
    let mut seen = std::collections::HashSet::new();
    match chat {
        Some(chat_id) => {
            let mut stmt = conn.prepare("SELECT chat_id, id FROM messages WHERE chat_id = ?1")?;
            let rows = stmt.query_map([chat_id], |row| {
                Ok(format!(
                    "{}\0{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            })?;
            for row in rows {
                seen.insert(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare("SELECT chat_id, id FROM messages")?;
            let rows = stmt.query_map([], |row| {
                Ok(format!(
                    "{}\0{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            })?;
            for row in rows {
                seen.insert(row?);
            }
        }
    }
    Ok(seen)
}

fn new_message_rows(
    data_dir: &Path,
    chat: Option<&str>,
    seen: &std::collections::HashSet<String>,
) -> Result<Vec<Value>> {
    let conn = open_existing_db(data_dir)?;
    let sql = match chat {
        Some(_) => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             WHERE chat_id = ?1
             ORDER BY created_at_secs ASC, id ASC"
        }
        None => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             ORDER BY created_at_secs ASC, id ASC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Value> {
        Ok(json!({
            "chat_id": row.get::<_, String>(0)?,
            "id": row.get::<_, String>(1)?,
            "body": row.get::<_, String>(2)?,
            "is_outgoing": row.get::<_, i64>(3)? != 0,
            "created_at_secs": row.get::<_, i64>(4)?,
            "delivery": row.get::<_, String>(5)?,
        }))
    };
    let mut messages = Vec::new();
    match chat {
        Some(chat_id) => {
            let rows = stmt.query_map([chat_id], map_row)?;
            for row in rows {
                let message = row?;
                let key = format!(
                    "{}\0{}",
                    message["chat_id"].as_str().unwrap_or_default(),
                    message["id"].as_str().unwrap_or_default()
                );
                if !seen.contains(&key) {
                    messages.push(message);
                }
            }
        }
        None => {
            let rows = stmt.query_map([], map_row)?;
            for row in rows {
                let message = row?;
                let key = format!(
                    "{}\0{}",
                    message["chat_id"].as_str().unwrap_or_default(),
                    message["id"].as_str().unwrap_or_default()
                );
                if !seen.contains(&key) {
                    messages.push(message);
                }
            }
        }
    }
    Ok(messages)
}

fn print_output(json_output: bool, command: &str, data: Value) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&Envelope {
                status: "ok",
                command: command.to_string(),
                data,
            })?
        );
    } else if data.is_object() || data.is_array() {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        println!("{data}");
    }
    Ok(())
}

fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Login { .. } => "login",
        Commands::Restore { .. } => "restore",
        Commands::Logout => "logout",
        Commands::Whoami => "whoami",
        Commands::State => "state",
        Commands::Sync { .. } => "sync",
        Commands::Search { .. } => "search",
        Commands::Tail { .. } => "tail",
        Commands::Listen { .. } => "listen",
        Commands::Account(_) => "account",
        Commands::Chat(_) => "chat",
        Commands::Send { .. } => "send",
        Commands::React { .. } => "react",
        Commands::Typing { .. } => "typing",
        Commands::Receipt { .. } => "receipt",
        Commands::Read { .. } => "read",
        Commands::Seen { .. } => "seen",
        Commands::Invite(_) => "invite",
        Commands::Link(_) => "link",
        Commands::Group(_) => "group",
        Commands::Relay(_) => "relay",
    }
}

fn account_json(account: &iris_chat_core::AccountSnapshot) -> Value {
    json!({
        "user_id": account.public_key_hex,
        "npub": account.npub,
        "name": account.display_name,
        "device_id": account.device_public_key_hex,
        "device_npub": account.device_npub,
        "has_secret_key": account.has_owner_signing_authority,
        "device_state": authorization_state(&account.authorization_state),
    })
}

fn state_json(state: &AppState) -> Value {
    json!({
        "account": state.account.as_ref().map(account_json),
        "chats": chat_list_json(state),
        "groups": group_list_json(state),
        "current_chat": state.current_chat.as_ref().map(|chat| chat_json(chat, usize::MAX)),
        "message_servers": state.preferences.nostr_relay_urls,
        "toast": state.toast,
    })
}

fn chat_list_json(state: &AppState) -> Vec<Value> {
    state.chat_list.iter().map(thread_json).collect()
}

fn group_list_json(state: &AppState) -> Vec<Value> {
    state
        .chat_list
        .iter()
        .filter(|thread| matches!(thread.kind, ChatKind::Group))
        .map(thread_json)
        .collect()
}

fn thread_json(thread: &ChatThreadSnapshot) -> Value {
    json!({
        "chat_id": thread.chat_id,
        "kind": chat_kind(&thread.kind),
        "name": thread.display_name,
        "subtitle": thread.subtitle,
        "member_count": thread.member_count,
        "last_message": thread.last_message_preview,
        "last_message_at_secs": thread.last_message_at_secs,
        "unread_count": thread.unread_count,
        "muted": thread.is_muted,
    })
}

fn chat_summary_json(chat: &CurrentChatSnapshot) -> Value {
    json!({
        "chat_id": chat.chat_id,
        "kind": chat_kind(&chat.kind),
        "name": chat.display_name,
        "group_id": chat.group_id,
        "member_count": chat.member_count,
        "message_count": chat.messages.len(),
        "message_ttl_seconds": chat.message_ttl_seconds,
        "muted": chat.is_muted,
    })
}

fn chat_json(chat: &CurrentChatSnapshot, limit: usize) -> Value {
    let skip = chat.messages.len().saturating_sub(limit);
    json!({
        "chat": chat_summary_json(chat),
        "messages": chat.messages.iter().skip(skip).map(message_json).collect::<Vec<_>>(),
    })
}

fn message_json(message: &ChatMessageSnapshot) -> Value {
    json!({
        "id": message.id,
        "chat_id": message.chat_id,
        "author": message.author,
        "body": message.body,
        "is_outgoing": message.is_outgoing,
        "created_at_secs": message.created_at_secs,
        "expires_at_secs": message.expires_at_secs,
        "delivery": delivery(&message.delivery),
        "source_event_id": message.source_event_id,
        "recipient_deliveries": message.recipient_deliveries.iter().map(|item| {
            json!({
                "owner_pubkey_hex": item.owner_pubkey_hex,
                "delivery": delivery(&item.delivery),
                "updated_at_secs": item.updated_at_secs,
            })
        }).collect::<Vec<_>>(),
        "delivery_trace": {
            "outer_event_ids": message.delivery_trace.outer_event_ids.clone(),
            "pending_relay_event_ids": message.delivery_trace.pending_relay_event_ids.clone(),
            "queued_protocol_targets": message.delivery_trace.queued_protocol_targets.clone(),
            "target_device_ids": message.delivery_trace.target_device_ids.clone(),
            "transport_channels": message.delivery_trace.transport_channels.clone(),
            "last_transport_error": message.delivery_trace.last_transport_error.clone(),
        },
        "attachments": message.attachments.iter().map(|item| {
            json!({
                "filename": item.filename,
                "nhash": item.nhash,
                "url": item.htree_url,
            })
        }).collect::<Vec<_>>(),
        "reactions": message.reactions.iter().map(|item| {
            json!({
                "emoji": item.emoji,
                "count": item.count,
                "reacted_by_me": item.reacted_by_me,
            })
        }).collect::<Vec<_>>(),
    })
}

fn group_json(group: &GroupDetailsSnapshot) -> Value {
    json!({
        "group_id": group.group_id,
        "name": group.name,
        "picture_url": group.picture_url,
        "can_manage": group.can_manage,
        "muted": group.is_muted,
        "revision": group.revision,
        "members": group.members.iter().map(|member| {
            json!({
                "user_id": member.owner_pubkey_hex,
                "name": member.display_name,
                "npub": member.npub,
                "admin": member.is_admin,
                "creator": member.is_creator,
                "me": member.is_local_owner,
            })
        }).collect::<Vec<_>>(),
    })
}

fn require_account(state: &AppState) -> Result<iris_chat_core::AccountSnapshot> {
    state
        .account
        .clone()
        .context("Create or restore a profile first.")
}

fn fail_on_toast(state: &AppState) -> Result<()> {
    if let Some(toast) = &state.toast {
        anyhow::bail!(toast.clone());
    }
    Ok(())
}

fn chat_action_input(state: &AppState, input: &str) -> String {
    resolve_chat_id(state, input).unwrap_or_else(|_| input.to_string())
}

fn resolve_chat_id(state: &AppState, input: &str) -> Result<String> {
    if let Some(chat) = &state.current_chat {
        if chat.chat_id == input || chat.group_id.as_deref() == Some(input) {
            return Ok(chat.chat_id.clone());
        }
    }
    state
        .chat_list
        .iter()
        .find(|thread| {
            thread.chat_id == input
                || thread.display_name.eq_ignore_ascii_case(input)
                || thread.subtitle.as_deref() == Some(input)
        })
        .map(|thread| thread.chat_id.clone())
        .with_context(|| format!("Chat not found: {input}"))
}

fn resolve_group_id(state: &AppState, input: &str) -> Result<String> {
    if let Some(group) = &state.group_details {
        if group.group_id == input || group.name.eq_ignore_ascii_case(input) {
            return Ok(group.group_id.clone());
        }
    }
    state
        .chat_list
        .iter()
        .find(|thread| {
            matches!(thread.kind, ChatKind::Group)
                && (thread.chat_id == input
                    || thread.chat_id == normalize_group_chat(input)
                    || thread.display_name.eq_ignore_ascii_case(input))
        })
        .map(|thread| {
            thread
                .chat_id
                .strip_prefix("group:")
                .unwrap_or(&thread.chat_id)
                .to_string()
        })
        .with_context(|| format!("Group not found: {input}"))
}

fn normalize_group_chat(group: &str) -> String {
    if group.starts_with("group:") {
        group.to_string()
    } else {
        format!("group:{group}")
    }
}

fn is_busy(state: &AppState) -> bool {
    let busy = &state.busy;
    busy.creating_account
        || busy.restoring_session
        || busy.linking_device
        || busy.creating_chat
        || busy.creating_group
        || busy.sending_message
        || busy.updating_roster
        || busy.updating_group
        || busy.creating_invite
        || busy.accepting_invite
        || busy.uploading_attachment
}

fn is_busy_or_syncing(state: &AppState) -> bool {
    is_busy(state) || state.busy.syncing_network
}

fn chat_kind(kind: &ChatKind) -> &'static str {
    match kind {
        ChatKind::Direct => "direct",
        ChatKind::Group => "group",
    }
}

fn delivery(delivery: &DeliveryState) -> &'static str {
    match delivery {
        DeliveryState::Queued => "queued",
        DeliveryState::Pending => "pending",
        DeliveryState::Sent => "sent",
        DeliveryState::Received => "received",
        DeliveryState::Seen => "seen",
        DeliveryState::Failed => "failed",
    }
}

fn authorization_state(state: &DeviceAuthorizationState) -> &'static str {
    match state {
        DeviceAuthorizationState::Authorized => "authorized",
        DeviceAuthorizationState::AwaitingApproval => "awaiting_approval",
        DeviceAuthorizationState::Revoked => "revoked",
    }
}

fn open_existing_db(data_dir: &Path) -> Result<Connection> {
    let path = data_dir.join("core.sqlite3");
    Connection::open(path).context("Open Iris chat database")
}

fn account_bundle_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cli-account.json")
}

fn read_account_bundle(data_dir: &Path) -> Result<Option<AccountBundle>> {
    let path = account_bundle_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read account bundle {}", path.display()))?;
    Ok(Some(serde_json::from_str(&raw)?))
}

fn write_account_bundle(data_dir: &Path, bundle: &AccountBundle) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = account_bundle_path(data_dir);
    std::fs::write(&path, serde_json::to_vec_pretty(bundle)?)
        .with_context(|| format!("write account bundle {}", path.display()))
}

fn default_data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Iris Chat CLI");
    }
    PathBuf::from(".iris-chat")
}
