use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use iris_chat_core::{
    AppAction, AppReconciler, AppState, AppUpdate, ChatKind, ChatMessageSnapshot,
    ChatThreadSnapshot, CurrentChatSnapshot, DeliveryState, DesktopNearbyObserver,
    DesktopNearbySnapshot, DeviceAuthorizationState, FfiApp, FfiDesktopNearby,
    GroupDetailsSnapshot,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

const TOP_LEVEL_HELP: &str = "\
Account:
  login      Use a secret key
  restore    Use a secret key
  logout     Forget this device
  whoami     Show this profile
  account    Profile tools

Messages:
  chat       Chat tools
  send       Send a message
  read       Show messages
  seen       Mark messages seen
  react      React to a message
  typing     Send typing status
  receipt    Send delivery status
  search     Search messages
  tail       Show recent messages
  listen     Watch for messages

Groups:
  group      Group chat tools

Invites and Devices:
  invite     Invite someone
  link       Link this device

Message Servers:
  relay      Message server tools

Maintenance:
  state      Show local state
  sync       Sync now
  privacy    Privacy tools
  update     Update iris
  help       Print help";

#[derive(Parser)]
#[command(name = "iris")]
#[command(version = env!("IRIS_APP_VERSION"))]
#[command(about = "Iris Chat command line client")]
#[command(after_help = TOP_LEVEL_HELP)]
#[command(help_template = "\
{before-help}{about-with-newline}
{usage-heading} {usage}{after-help}

Options:
{options}")]
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
    #[command(flatten)]
    Account(AccountTopCommands),
    #[command(flatten)]
    Messages(MessageTopCommands),
    #[command(flatten)]
    Groups(GroupTopCommands),
    #[command(flatten)]
    InvitesAndDevices(InviteDeviceTopCommands),
    #[command(flatten)]
    MessageServers(MessageServerTopCommands),
    #[command(subcommand)]
    Privacy(PrivacyCommands),
    #[command(flatten)]
    Maintenance(MaintenanceTopCommands),
}

#[derive(Subcommand)]
enum AccountTopCommands {
    Login {
        secret_key: String,
    },
    Restore {
        secret_key: String,
    },
    Logout,
    Whoami,
    #[command(subcommand)]
    Account(AccountCommands),
}

#[derive(Subcommand)]
enum MessageTopCommands {
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
    Read {
        chat: String,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    Seen {
        chat: String,
        message_ids: Vec<String>,
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
}

#[derive(Subcommand)]
enum GroupTopCommands {
    #[command(subcommand)]
    Group(GroupCommands),
}

#[derive(Subcommand)]
enum InviteDeviceTopCommands {
    #[command(subcommand)]
    Invite(InviteCommands),
    #[command(subcommand)]
    Link(LinkCommands),
}

#[derive(Subcommand)]
enum MessageServerTopCommands {
    #[command(subcommand)]
    Relay(RelayCommands),
}

#[derive(Subcommand)]
enum MaintenanceTopCommands {
    State,
    Sync {
        #[arg(long, default_value_t = 1500)]
        wait_ms: u64,
    },
    /// Check for and install a newer iris binary published via hashtree
    #[command(subcommand)]
    Update(UpdateCommands),
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

#[derive(Subcommand)]
enum PrivacyCommands {
    UnknownUsers { mode: UnknownUsersMode },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum UnknownUsersMode {
    Allow,
    Block,
}

/// Resolve, download, and install iris updates by shelling out to the
/// published `htree update` CLI (from `hashtree-cli`). We don't link the
/// updater library directly because iris-chat-rs pins hashtree-core 0.2.8
/// while the updater needs 0.2.45+.
#[derive(Subcommand)]
enum UpdateCommands {
    /// Print the latest published version and the asset that would be picked
    Check,
    /// Download the matching asset to a path (defaults to alongside the
    /// running binary)
    Download {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Replace the running binary with the newer one
    Install {
        /// Override the install destination (defaults to current_exe())
        #[arg(long)]
        to: Option<PathBuf>,
        /// Override the asset kind (defaults to inferred from filename)
        #[arg(long)]
        kind: Option<String>,
        /// Skip if the published version is not newer than current
        #[arg(long)]
        only_if_newer: bool,
    },
}

const IRIS_UPDATE_REFERENCE: &str =
    "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest";

fn run_iris_update(cmd: &UpdateCommands) -> Result<()> {
    let mut args = vec!["install".to_string(), IRIS_UPDATE_REFERENCE.to_string()];
    let current_version = env!("IRIS_APP_VERSION").to_string();
    args.extend(["--current-version".into(), current_version]);
    match cmd {
        UpdateCommands::Check => {
            args.push("--check".into());
        }
        UpdateCommands::Download { out } => {
            args.push("--download-only".into());
            if let Some(out) = out {
                args.extend(["--to".into(), out.display().to_string()]);
            }
        }
        UpdateCommands::Install {
            to,
            kind,
            only_if_newer,
        } => {
            let dest = match to {
                Some(p) => p.clone(),
                None => std::env::current_exe()
                    .context("could not determine current_exe() for install destination")?,
            };
            args.extend([
                "--to".into(),
                dest.display().to_string(),
                "--executable".into(),
            ]);
            if let Some(kind) = kind {
                args.extend(["--kind".into(), kind.clone()]);
            }
            if *only_if_newer {
                args.push("--only-if-newer".into());
            }
        }
    }

    let status = std::process::Command::new("htree")
        .args(&args)
        .status()
        .context(
            "failed to spawn htree (install hashtree-cli with `cargo install hashtree-cli`)",
        )?;
    if !status.success() {
        anyhow::bail!("htree {} exited with status {status}", args.join(" "));
    }
    Ok(())
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
        Commands::Messages(MessageTopCommands::Search { query, limit }) => {
            search_messages(&data_dir, &query, limit)?
        }
        Commands::Messages(MessageTopCommands::Tail {
            limit,
            follow,
            chat,
            interval_ms,
        }) => {
            if follow {
                follow_messages(&data_dir, chat.as_deref(), interval_ms, "tail")?;
                return Ok(());
            }
            tail_messages(&data_dir, limit, chat.as_deref())?
        }
        Commands::Messages(MessageTopCommands::Listen {
            chat,
            interval_ms,
            nearby_lan,
        }) => {
            listen(&data_dir, chat.as_deref(), interval_ms, nearby_lan)?;
            return Ok(());
        }
        command => {
            let cli_app = CliApp::open(&data_dir)?;
            let data = handle_command(&cli_app, &data_dir, command)?;
            let background_sync = should_spawn_background_sync(&cli_app.app.state(), &data);
            cli_app.app.shutdown();
            drop(cli_app);
            print_output(cli.json, &command_name, data)?;
            if background_sync {
                spawn_background_sync(&data_dir);
            }
            return Ok(());
        }
    };
    print_output(cli.json, &command_name, data)
}

impl CliApp {
    fn open(data_dir: &Path) -> Result<Self> {
        let app = FfiApp::new(
            data_dir.to_string_lossy().to_string(),
            String::new(),
            env!("IRIS_APP_VERSION").to_string(),
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
            cli_app.wait_for_restored_account(Duration::from_secs(6))?;
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

    fn wait_for_network_runtime_ready(
        &self,
        timeout: Duration,
        require_protocol_idle: bool,
    ) -> Result<AppState> {
        let started = Instant::now();
        let mut last_state = self.app.state();
        while started.elapsed() < timeout {
            last_state = self.app.state();
            fail_on_toast(&last_state)?;
            if self.network_runtime_ready(&last_state, require_protocol_idle) {
                return Ok(last_state);
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(last_state)
    }

    fn network_runtime_ready(&self, state: &AppState, require_protocol_idle: bool) -> bool {
        if is_busy(state) || state.busy.syncing_network {
            return false;
        }
        if require_protocol_idle && has_pending_runtime_publishes(state) {
            return false;
        }
        if state.preferences.nostr_relay_urls.is_empty() {
            return true;
        }
        let Ok(bundle) = serde_json::from_str::<Value>(&self.app.export_support_bundle_json())
        else {
            return false;
        };
        let relay_connected = bundle
            .pointer("/relay_transport/connected_relay_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0;
        let subscription = bundle
            .get("protocol_subscription")
            .cloned()
            .unwrap_or(Value::Null);
        if !relay_connected {
            return false;
        }
        let refresh_in_flight = subscription
            .get("refresh_in_flight")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let refresh_dirty = subscription
            .get("refresh_dirty")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let desired = subscription
            .get("desired_plan")
            .cloned()
            .unwrap_or(Value::Null);
        let applied = subscription
            .get("applied_plan")
            .cloned()
            .unwrap_or(Value::Null);
        let applying = subscription
            .get("applying_plan")
            .cloned()
            .unwrap_or(Value::Null);
        if !require_protocol_idle {
            return desired.is_null() || desired == applied || desired == applying;
        }
        if refresh_in_flight || refresh_dirty || desired != applied {
            return false;
        }
        if require_protocol_idle {
            let pending_inbound = bundle
                .pointer("/protocol_engine/pending_inbound_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let pending_group = bundle
                .pointer("/protocol_engine/pending_group_sender_key_message_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if pending_inbound > 0 || pending_group > 0 {
                return false;
            }
        }
        true
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

    fn wait_for_restored_account(&self, timeout: Duration) -> Result<AppState> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            let state = self.app.state();
            fail_on_toast(&state)?;
            if state.account.is_some() && !state.busy.restoring_session {
                return Ok(state);
            }
            thread::sleep(Duration::from_millis(20));
        }
        let state = self.app.state();
        fail_on_toast(&state)?;
        anyhow::bail!("Timed out restoring account bundle.")
    }
}

fn handle_command(cli: &CliApp, data_dir: &Path, command: Commands) -> Result<Value> {
    match command {
        Commands::Account(command) => handle_account_top_command(cli, data_dir, command),
        Commands::Messages(command) => handle_message_top_command(cli, command),
        Commands::Groups(GroupTopCommands::Group(command)) => handle_group_command(cli, command),
        Commands::InvitesAndDevices(command) => handle_invite_device_command(cli, command),
        Commands::MessageServers(MessageServerTopCommands::Relay(command)) => {
            handle_relay_command(cli, command)
        }
        Commands::Privacy(command) => handle_privacy_command(cli, command),
        Commands::Maintenance(command) => handle_maintenance_command(cli, command),
    }
}

fn handle_account_top_command(
    cli: &CliApp,
    data_dir: &Path,
    command: AccountTopCommands,
) -> Result<Value> {
    match command {
        AccountTopCommands::Login { secret_key } | AccountTopCommands::Restore { secret_key } => {
            restore_account(cli, &secret_key)
        }
        AccountTopCommands::Logout => {
            remove_account_bundle(data_dir)?;
            cli.dispatch_and_wait(AppAction::Logout, Duration::from_secs(2))?;
            Ok(json!({ "logged_out": true }))
        }
        AccountTopCommands::Whoami => Ok(account_json(&require_account(&cli.app.state())?)),
        AccountTopCommands::Account(command) => handle_account_command(cli, data_dir, command),
    }
}

fn handle_account_command(
    cli: &CliApp,
    data_dir: &Path,
    command: AccountCommands,
) -> Result<Value> {
    match command {
        AccountCommands::Create { name } => {
            cli.dispatch_and_wait_network(
                AppAction::CreateAccount { name },
                Duration::from_secs(8),
            )?;
            let _ = cli.wait_for_network_runtime_ready(Duration::from_secs(4), true)?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            Ok(account_json(&require_account(&state)?))
        }
        AccountCommands::Restore { secret_key } => restore_account(cli, &secret_key),
        AccountCommands::Bundle => {
            let bundle = read_account_bundle(data_dir)?.context("No saved account bundle.")?;
            Ok(json!({
                "owner_pubkey_hex": bundle.owner_pubkey_hex,
                "has_owner_secret": bundle.owner_nsec.is_some(),
                "has_device_secret": !bundle.device_nsec.is_empty(),
            }))
        }
    }
}

fn handle_message_top_command(cli: &CliApp, command: MessageTopCommands) -> Result<Value> {
    match command {
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
            open_chat(cli, &chat).map(|chat| chat_json(&chat, limit))
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

fn handle_chat_command(cli: &CliApp, command: ChatCommands) -> Result<Value> {
    match command {
        ChatCommands::List => Ok(json!({ "chats": chat_list_json(&cli.app.state()) })),
        ChatCommands::Create { user_id } => create_chat(cli, &user_id),
        ChatCommands::Open { chat } => {
            open_chat(cli, &chat).map(|chat| chat_json(&chat, usize::MAX))
        }
        ChatCommands::Read { chat, limit } => {
            open_chat(cli, &chat).map(|chat| chat_json(&chat, limit))
        }
        ChatCommands::Send {
            chat,
            message,
            ttl,
            expires_at,
        } => {
            let expires_at = message_expiration(ttl, expires_at)?;
            send_message(cli, &chat, &message, expires_at)
        }
        ChatCommands::Seen { chat, message_ids } => mark_seen(cli, &chat, message_ids),
        ChatCommands::Delete { chat } => {
            let chat_id = resolve_chat_id(&cli.app.state(), &chat)?;
            cli.dispatch_and_wait(
                AppAction::DeleteChat {
                    chat_id: chat_id.clone(),
                },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "chat_id": chat_id, "deleted": true }))
        }
        ChatCommands::Ttl { chat, seconds } => {
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
        ChatCommands::Mute { chat, muted } => {
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
    }
}

fn handle_invite_device_command(cli: &CliApp, command: InviteDeviceTopCommands) -> Result<Value> {
    match command {
        InviteDeviceTopCommands::Invite(InviteCommands::Create) => {
            cli.dispatch_and_wait(AppAction::CreatePublicInvite, Duration::from_secs(3))?;
            let state = cli.app.state();
            fail_on_toast(&state)?;
            let invite = state.public_invite.context("No invite was created.")?;
            Ok(json!({ "url": invite.url }))
        }
        InviteDeviceTopCommands::Invite(InviteCommands::Accept { invite }) => {
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
        InviteDeviceTopCommands::Link(LinkCommands::Accept { invite }) => {
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
    }
}

fn handle_group_command(cli: &CliApp, command: GroupCommands) -> Result<Value> {
    match command {
        GroupCommands::Create { name, members } => {
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
        GroupCommands::List => Ok(json!({ "groups": group_list_json(&cli.app.state()) })),
        GroupCommands::Show { group } => show_group(cli, &group),
        GroupCommands::Send { group, message } => {
            send_message(cli, &normalize_group_chat(&group), &message, None)
        }
        GroupCommands::Read { group, limit } => {
            open_chat(cli, &normalize_group_chat(&group)).map(|chat| chat_json(&chat, limit))
        }
        GroupCommands::Add { group, members } => {
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
        GroupCommands::Remove { group, members } => {
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
        GroupCommands::AddAdmin { group, member } => set_group_admin(cli, &group, &member, true),
        GroupCommands::RemoveAdmin { group, member } => {
            set_group_admin(cli, &group, &member, false)
        }
        GroupCommands::Rename { group, name } => {
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
        GroupCommands::React {
            group,
            message_id,
            emoji,
        } => react(cli, &normalize_group_chat(&group), &message_id, &emoji),
        GroupCommands::Delete { group } => {
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
    }
}

fn handle_relay_command(cli: &CliApp, command: RelayCommands) -> Result<Value> {
    match command {
        RelayCommands::List => {
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        RelayCommands::Add { url } => {
            cli.dispatch_and_wait(
                AppAction::AddNostrRelay { relay_url: url },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        RelayCommands::Remove { url } => {
            cli.dispatch_and_wait(
                AppAction::RemoveNostrRelay { relay_url: url },
                Duration::from_secs(2),
            )?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        RelayCommands::Reset => {
            cli.dispatch_and_wait(AppAction::ResetNostrRelays, Duration::from_secs(2))?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
        RelayCommands::Set { urls } => {
            cli.dispatch_and_wait(
                AppAction::SetNostrRelays { relay_urls: urls },
                Duration::from_secs(3),
            )?;
            Ok(json!({ "message_servers": cli.app.state().preferences.nostr_relay_urls }))
        }
    }
}

fn handle_privacy_command(cli: &CliApp, command: PrivacyCommands) -> Result<Value> {
    match command {
        PrivacyCommands::UnknownUsers { mode } => {
            let accept = matches!(mode, UnknownUsersMode::Allow);
            cli.dispatch_and_wait(
                AppAction::SetAcceptUnknownDirectMessages { enabled: accept },
                Duration::from_secs(2),
            )?;
            Ok(json!({
                "unknown_users": if accept { "allow" } else { "block" },
                "accept_unknown_direct_messages": cli.app.state().preferences.accept_unknown_direct_messages,
            }))
        }
    }
}

fn handle_maintenance_command(cli: &CliApp, command: MaintenanceTopCommands) -> Result<Value> {
    match command {
        MaintenanceTopCommands::State => Ok(state_json(&cli.app.state())),
        MaintenanceTopCommands::Sync { wait_ms } => {
            let timeout = Duration::from_millis(wait_ms.max(100));
            let _ = cli.dispatch_and_wait_network(AppAction::AppForegrounded, timeout)?;
            let state = cli.wait_for_network_runtime_ready(timeout, true)?;
            Ok(state_json(&state))
        }
        MaintenanceTopCommands::Update(cmd) => {
            run_iris_update(&cmd)?;
            Ok(json!({ "ok": true }))
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
    cli.dispatch_and_wait(action, Duration::from_secs(2))?;
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
         ORDER BY created_at_secs DESC, rowid DESC
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
             ORDER BY created_at_secs DESC, rowid DESC
             LIMIT ?2"
        }
        None => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             ORDER BY created_at_secs DESC, rowid DESC
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
    let _ = cli.dispatch_and_wait_network(AppAction::AppForegrounded, Duration::from_secs(8))?;
    let state = cli.wait_for_network_runtime_ready(Duration::from_secs(25), true)?;
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
    loop {
        thread::sleep(interval);
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

fn should_spawn_background_sync(state: &AppState, data: &Value) -> bool {
    !state.preferences.nostr_relay_urls.is_empty()
        && data
            .get("is_outgoing")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && data.get("id").and_then(Value::as_str).is_some()
        && data
            .get("delivery")
            .and_then(Value::as_str)
            .is_some_and(|delivery| matches!(delivery, "queued" | "pending"))
}

fn spawn_background_sync(data_dir: &Path) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    #[cfg(unix)]
    {
        let script = r#"sleep 0.2
for _ in 1 2 3; do
  "$1" --data-dir "$2" sync --wait-ms 8000 >/dev/null 2>&1 && exit 0
  sleep 0.5
done
exit 0
"#;
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("iris-background-sync")
            .arg(exe)
            .arg(data_dir)
            .env("IRIS_CLI_BACKGROUND_SYNC", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        return;
    }

    #[cfg(not(unix))]
    let _ = std::process::Command::new(exe)
        .arg("--data-dir")
        .arg(data_dir)
        .args(["sync", "--wait-ms", "8000"])
        .env("IRIS_CLI_BACKGROUND_SYNC", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Account(command) => match command {
            AccountTopCommands::Login { .. } => "login",
            AccountTopCommands::Restore { .. } => "restore",
            AccountTopCommands::Logout => "logout",
            AccountTopCommands::Whoami => "whoami",
            AccountTopCommands::Account(_) => "account",
        },
        Commands::Messages(command) => match command {
            MessageTopCommands::Chat(_) => "chat",
            MessageTopCommands::Send { .. } => "send",
            MessageTopCommands::Read { .. } => "read",
            MessageTopCommands::Seen { .. } => "seen",
            MessageTopCommands::React { .. } => "react",
            MessageTopCommands::Typing { .. } => "typing",
            MessageTopCommands::Receipt { .. } => "receipt",
            MessageTopCommands::Search { .. } => "search",
            MessageTopCommands::Tail { .. } => "tail",
            MessageTopCommands::Listen { .. } => "listen",
        },
        Commands::Groups(_) => "group",
        Commands::InvitesAndDevices(command) => match command {
            InviteDeviceTopCommands::Invite(_) => "invite",
            InviteDeviceTopCommands::Link(_) => "link",
        },
        Commands::MessageServers(_) => "relay",
        Commands::Privacy(_) => "privacy",
        Commands::Maintenance(command) => match command {
            MaintenanceTopCommands::State => "state",
            MaintenanceTopCommands::Sync { .. } => "sync",
            MaintenanceTopCommands::Update(_) => "update",
        },
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
        "accept_unknown_direct_messages": state.preferences.accept_unknown_direct_messages,
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
        "pinned": thread.is_pinned,
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
    is_busy(state) || state.busy.syncing_network || has_pending_runtime_publishes(state)
}

fn has_pending_runtime_publishes(state: &AppState) -> bool {
    state
        .network_status
        .as_ref()
        .is_some_and(|status| status.pending_outbound_count > 0)
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

fn remove_account_bundle(data_dir: &Path) -> Result<()> {
    let path = account_bundle_path(data_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("remove account bundle {}", path.display()));
        }
    }
    if read_account_bundle(data_dir)?.is_some() {
        anyhow::bail!("account bundle still present after delete");
    }
    Ok(())
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
