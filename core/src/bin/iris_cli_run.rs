use super::*;

pub(super) fn run(cli: Cli) -> Result<()> {
    if super::iris_service::route(&cli)? {
        return Ok(());
    }
    let Cli {
        json: json_output,
        data_dir,
        no_background_sync,
        command,
    } = cli;
    let command = match command {
        Commands::Maintenance(MaintenanceTopCommands::Update(cmd)) => {
            return run_iris_update(cmd, json_output);
        }
        other => other,
    };

    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    ensure_private_data_dir(&data_dir)?;
    let command_name = command_name(&command).to_string();
    let data = match command {
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
            let background_sync =
                !no_background_sync && should_spawn_background_sync(&cli_app.app.state(), &data);
            cli_app.app.shutdown();
            drop(cli_app);
            print_output(json_output, &command_name, data)?;
            if background_sync {
                spawn_background_sync(&data_dir);
            }
            return Ok(());
        }
    };
    print_output(json_output, &command_name, data)
}
