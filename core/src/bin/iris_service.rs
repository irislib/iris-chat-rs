//! A single long-lived core owns the profile; all clients share that runtime.
use super::iris_message_rows::new_message_rows;
use super::iris_service_transport::{self as transport, Connection};
use super::*;
use interprocess::local_socket::{prelude::*, Stream};
use std::{collections::HashSet, io::ErrorKind};

#[derive(Subcommand)]
pub(super) enum ServiceCommands {
    /// Keep this profile online and serve local commands until stopped.
    Run {
        #[arg(long)]
        nearby_lan: bool,
    },
    /// Show service and message-server health.
    Status,
    /// Stop the service after any current command finishes.
    Stop,
}

pub(super) fn route(cli: &Cli) -> Result<bool> {
    if matches!(
        cli.command,
        Commands::Maintenance(MaintenanceTopCommands::Update(_))
    ) {
        return Ok(false);
    }
    let data_dir = cli.data_dir.clone().unwrap_or_else(default_data_dir);
    if let Commands::Maintenance(MaintenanceTopCommands::Service(ServiceCommands::Run {
        nearby_lan,
    })) = &cli.command
    {
        ensure_private_data_dir(&data_dir)?;
        serve(&data_dir.canonicalize()?, *nearby_lan)?;
        return Ok(true);
    }
    let is_service = matches!(
        cli.command,
        Commands::Maintenance(MaintenanceTopCommands::Service(_))
    );
    let required = is_service || std::env::var_os("IRIS_REQUIRE_SERVICE").is_some_and(|v| v == "1");
    if !data_dir.exists() {
        anyhow::ensure!(!required, "Iris service is not running for this profile");
        return Ok(false);
    }
    let data_dir = data_dir.canonicalize()?;
    let stream = match Stream::connect(transport::name(&data_dir)?) {
        Ok(s) => s,
        Err(e)
            if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused)
                && !required =>
        {
            return Ok(false)
        }
        Err(e) => return Err(e).context("Cannot connect to Iris service"),
    };
    transport::verify_peer(&stream, &data_dir)?;
    let mut connection = Connection::new(stream)?;
    // Encode arguments, never shell text; the service parses the same CLI grammar.
    let args: Vec<String> = std::env::args().skip(1).collect();
    connection.queue(&json!({"version":1,"profile":data_dir,"args":args}))?;
    let mut last_frame = Instant::now();
    loop {
        connection.flush()?;
        anyhow::ensure!(
            last_frame.elapsed() < Duration::from_secs(300),
            "Iris service timed out; check delivery before retrying a send"
        );
        if let Some(frame) = connection.receive(32 * 1024 * 1024)? {
            last_frame = Instant::now();
            match frame["type"].as_str() {
                Some("result") => {
                    print_output(cli.json, command_name(&cli.command), frame["data"].clone())?;
                    return Ok(true);
                }
                Some("error") => anyhow::bail!(
                    "{}",
                    frame["error"].as_str().unwrap_or("Service command failed")
                ),
                Some("stream") => print_stream_envelope(
                    frame["command"].as_str().unwrap_or("message"),
                    frame["data"].clone(),
                )?,
                Some("heartbeat") => (),
                _ => anyhow::bail!("Unsupported Iris service response"),
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}

struct Subscription {
    chat: Option<String>,
    owner: String,
    seen: HashSet<String>,
    interval: Duration,
    next_poll: Instant,
}
struct Client {
    connection: Connection,
    accepted: Instant,
    done: bool,
    subscription: Option<Subscription>,
}

fn serve(data_dir: &Path, nearby_lan: bool) -> Result<()> {
    // Acquire the unchanged core lock BEFORE reclaiming a stale endpoint.
    let cli = CliApp::open(data_dir)?;
    let listener = transport::bind(data_dir)?;
    cli.app.dispatch(AppAction::AppForegrounded);
    let _nearby = if nearby_lan {
        let service = FfiDesktopNearby::new(cli.app.clone(), Box::new(CliNearbyObserver));
        service.start("Iris".to_string());
        Some(service)
    } else {
        None
    };
    print_stream_envelope("service", json!({"ready":true,"pid":std::process::id()}))?;
    let mut clients: Vec<Client> = Vec::new();
    let mut stopping = false;
    let result = (|| -> Result<()> {
        while !stopping || clients.iter().any(|c| c.done) {
            if !stopping {
                for _ in 0..8 {
                    match listener.accept() {
                        Ok(stream) if clients.len() < 32 => {
                            if transport::verify_peer(&stream, data_dir).is_ok() {
                                clients.push(Client {
                                    connection: Connection::new(stream)?,
                                    accepted: Instant::now(),
                                    done: false,
                                    subscription: None,
                                });
                            }
                        }
                        Ok(_) => (),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            clients.retain_mut(|client| {
                match poll_client(client, &cli, data_dir, nearby_lan, &mut stopping) {
                    Ok(keep) => keep,
                    Err(error) => {
                        // Best effort error; never log message bodies or command arguments.
                        let _ = client
                            .connection
                            .queue(&json!({"type":"error","error":error.to_string()}));
                        let _ = client.connection.flush();
                        false
                    }
                }
            });
            // The reconciler retains updates for short commands; a daemon must
            // not retain every incoming event for its entire lifetime.
            if let Ok(mut updates) = cli.reconciler.updates.lock() {
                updates.clear();
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })();
    // Close subscribers before releasing the profile guard.
    drop(clients);
    drop(listener);
    cli.app.shutdown();
    result
}

fn poll_client(
    client: &mut Client,
    cli: &CliApp,
    data_dir: &Path,
    nearby_lan: bool,
    stopping: &mut bool,
) -> Result<bool> {
    if client.done {
        return Ok(!client.connection.flush()?);
    }
    if *stopping {
        return Ok(false);
    }
    if let Some(subscription) = &mut client.subscription {
        if Instant::now() >= subscription.next_poll {
            // Reading durable rows makes slow clients and command latency lossless within
            // this subscription. Reconnecting callers reconcile from their own durable IDs.
            for message in new_message_rows(
                data_dir,
                subscription.chat.as_deref(),
                &subscription.seen,
                &subscription.owner,
            )? {
                let key = format!(
                    "{}\0{}",
                    message["chat_id"].as_str().unwrap_or_default(),
                    message["id"].as_str().unwrap_or_default()
                );
                client
                    .connection
                    .queue(&json!({"type":"stream","command":"message","data":message}))?;
                subscription.seen.insert(key);
            }
            client.connection.queue(&json!({"type":"heartbeat"}))?;
            subscription.next_poll = Instant::now() + subscription.interval;
        }
        client.connection.flush()?;
        return Ok(true);
    }
    anyhow::ensure!(
        client.accepted.elapsed() < Duration::from_secs(5),
        "Service request timed out"
    );
    let Some(request) = client.connection.receive(1024 * 1024)? else {
        return Ok(true);
    };
    anyhow::ensure!(
        request["version"] == 1,
        "Unsupported Iris service protocol version"
    );
    let args: Vec<String> = serde_json::from_value(request["args"].clone())?;
    let parsed = Cli::try_parse_from(std::iter::once("iris".to_string()).chain(args))?;
    // Resolve the profile in the client, whose cwd and IRIS_DATA_DIR may differ.
    // Commands always execute against this already-open core, never the parsed path.
    let requested_dir: PathBuf = serde_json::from_value(request["profile"].clone())?;
    anyhow::ensure!(
        requested_dir == data_dir,
        "A service command cannot change profiles"
    );
    let response = match parsed.command {
        Commands::Maintenance(MaintenanceTopCommands::Service(ServiceCommands::Status)) => {
            let state = cli.app.state();
            let support: Value = serde_json::from_str(&cli.app.export_support_bundle_json())?;
            json!({"running":true,"pid":std::process::id(),"account_loaded":state.account.is_some(),
                "network_ready":cli.network_runtime_ready(&state, false),
                "connected_servers":support.pointer("/relay_transport/connected_relay_count").cloned().unwrap_or(json!(0)),
                "nearby_lan":nearby_lan})
        }
        Commands::Maintenance(MaintenanceTopCommands::Service(ServiceCommands::Stop)) => {
            *stopping = true;
            json!({"stopping":true})
        }
        Commands::Maintenance(
            MaintenanceTopCommands::Service(ServiceCommands::Run { .. })
            | MaintenanceTopCommands::Update(_),
        ) => anyhow::bail!("Stop the existing service before starting or updating it"),
        Commands::Messages(MessageTopCommands::Listen {
            chat,
            interval_ms,
            nearby_lan: requested_nearby,
        }) => {
            anyhow::ensure!(
                !requested_nearby || nearby_lan,
                "Start the service with --nearby-lan first"
            );
            subscribe(
                client,
                cli,
                data_dir,
                chat.as_deref(),
                interval_ms,
                "listen",
                true,
            )?;
            return Ok(true);
        }
        Commands::Messages(MessageTopCommands::Tail {
            follow: true,
            chat,
            interval_ms,
            ..
        }) => {
            subscribe(
                client,
                cli,
                data_dir,
                chat.as_deref(),
                interval_ms,
                "tail",
                true,
            )?;
            return Ok(true);
        }
        Commands::Messages(MessageTopCommands::Tail { limit, chat, .. }) => {
            tail_messages(data_dir, limit, chat.as_deref())?
        }
        Commands::Messages(MessageTopCommands::Search { query, limit }) => {
            search_messages(data_dir, &query, limit)?
        }
        command => handle_command(cli, data_dir, command)?,
    };
    client
        .connection
        .queue(&json!({"type":"result","data":response}))?;
    client.done = true;
    Ok(true)
}

fn subscribe(
    client: &mut Client,
    cli: &CliApp,
    data_dir: &Path,
    chat: Option<&str>,
    interval_ms: u64,
    command: &str,
    network: bool,
) -> Result<()> {
    let state = cli.app.state();
    let owner = require_account(&state)?.public_key_hex;
    let chat = normalize_chat_filter(&state, chat);
    let seen = latest_message_keys(data_dir, chat.as_deref(), &owner)?;
    client.connection.queue(&json!({"type":"stream","command":command,"data":{"ready":true,"chat":chat,"network":network}}))?;
    client.subscription = Some(Subscription {
        chat,
        owner,
        seen,
        interval: Duration::from_millis(interval_ms.clamp(100, 60_000)),
        next_poll: Instant::now(),
    });
    Ok(())
}
