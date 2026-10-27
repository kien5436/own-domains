use crate::platform::host_installer;
#[cfg(target_os = "macos")]
use crate::privilege::setup_user;
use clap::{Parser, Subcommand};
use own_domains_client::{Client, ClientError, LifecycleError, connect_or_launch, stop_and_wait};
use own_domains_core::{ApplyStatus, DomainService, MappingInput, MappingPatch, SettingsStore};
use own_domains_daemon::{
    DaemonState, ProcessLock, UnavailableRuntimeBackend,
    platform::{SetupApplyResult, SetupFailure, SetupPreview, SetupRequest, UninstallRequest},
};
use own_domains_protocol::{Command, CommandResult, Endpoint, ProtocolError};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Debug, Parser)]
#[command(name = "own-domains", version, about = "Manage local .own domains")]
pub struct Cli {
    #[arg(long, global = true, default_value = ".own-domains")]
    pub state_dir: PathBuf,
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,
    #[arg(long, global = true)]
    pub pipe: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: CliCommand,
}

impl Cli {
    pub fn try_parse_from<I, T>(arguments: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::try_parse_from(arguments)
    }
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    Start,
    Stop,
    Status,
    List,
    Add {
        hostname: String,
        #[arg(long)]
        port: u32,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        https: bool,
    },
    Edit {
        hostname: String,
        #[arg(long)]
        port: Option<u32>,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        https: Option<bool>,
    },
    Enable {
        hostname: String,
        #[arg(long)]
        revision: u64,
    },
    Disable {
        hostname: String,
        #[arg(long)]
        revision: u64,
    },
    Remove {
        hostname: String,
        #[arg(long)]
        revision: u64,
    },
    Doctor,
    Setup {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        lan_address: String,
        #[arg(long)]
        interface: String,
    },
    Uninstall {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        purge: bool,
    },
}

pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    fn success(stdout: String) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        }
    }
    fn error(exit_code: i32, error: impl ToString) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr: format!("{}\n", error.to_string()),
        }
    }
}

pub fn execute(cli: Cli) -> CommandOutput {
    if let CliCommand::Setup {
        apply,
        lan_address,
        interface,
    } = &cli.command
    {
        return execute_setup(*apply, &cli.state_dir, lan_address, interface);
    }
    if let CliCommand::Uninstall { apply, purge } = &cli.command {
        return execute_uninstall(*apply, *purge, &cli.state_dir);
    }
    let endpoint = match (&cli.socket, &cli.pipe) {
        (Some(_), Some(_)) => return CommandOutput::error(2, "use either --socket or --pipe"),
        (_, Some(name)) => match Endpoint::named_pipe(name.clone()) {
            Ok(endpoint) => endpoint,
            Err(message) => return CommandOutput::error(2, message),
        },
        (Some(path), None) => Endpoint::unix(path.clone()),
        (None, None) => match Endpoint::default_for_state_dir(&cli.state_dir) {
            Ok(endpoint) => endpoint,
            Err(message) => return CommandOutput::error(2, message),
        },
    };
    let client_timeout = if matches!(cli.command, CliCommand::Start) {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(5)
    };
    match Client::with_timeout(&endpoint, client_timeout) {
        Ok(client) if matches!(cli.command, CliCommand::Stop) => {
            match stop_and_wait(&client, &endpoint, Duration::from_secs(5)) {
                Ok(()) => render(
                    CommandResult::Runtime(own_domains_protocol::RuntimeStatus::Stopped),
                    cli.json,
                    None,
                ),
                Err(LifecycleError::Client(error)) => client_error(error),
                Err(error) => CommandOutput::error(5, error),
            }
        }
        Ok(client) => run_live(&client, &cli.command, cli.json),
        Err(ClientError::DaemonUnavailable { .. }) => run_offline(&cli, &endpoint),
        Err(error) => client_error(error),
    }
}

fn render_preview(preview: &SetupPreview) -> String {
    preview
        .actions
        .iter()
        .map(|action| format!("{} {}", action.kind, action.path.display()))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn render_apply(result: &SetupApplyResult) -> String {
    result
        .written
        .iter()
        .map(|path| format!("written {}", path.display()))
        .chain(
            result
                .unchanged
                .iter()
                .map(|path| format!("unchanged {}", path.display())),
        )
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn setup_failure(error: SetupFailure) -> CommandOutput {
    if error.unresolved.is_empty() {
        CommandOutput::error(error.exit_code, error.message)
    } else {
        CommandOutput::error(
            error.exit_code,
            format!(
                "{}; unresolved: {}",
                error.message,
                error.unresolved.join(", ")
            ),
        )
    }
}

fn execute_setup(
    apply: bool,
    state_directory: &Path,
    lan_address: &str,
    interface: &str,
) -> CommandOutput {
    if !cfg!(target_os = "macos") && !cfg!(target_os = "windows") {
        return CommandOutput::error(3, "macOS host setup is unsupported on this platform");
    }
    #[cfg(target_os = "windows")]
    if apply && !crate::platform::windows::is_elevated() {
        return CommandOutput::error(2, "setup --apply must run from an elevated prompt");
    }
    #[cfg(target_os = "macos")]
    let (_, identity) = match setup_user(apply) {
        Ok(user) => user,
        Err(error) => return CommandOutput::error(2, error),
    };
    #[cfg(target_os = "macos")]
    if apply && identity.is_none() {
        return CommandOutput::error(2, "setup --apply must run as root through sudo");
    }
    let request = SetupRequest {
        state_directory: state_directory.to_path_buf(),
        lan_address: lan_address.to_string(),
        interface: interface.to_string(),
    };
    let installer = host_installer();
    if !apply {
        return match installer.preview_setup(&request) {
            Ok(preview) => CommandOutput::success(render_preview(&preview)),
            Err(error) => setup_failure(error),
        };
    }
    match installer.apply_setup(&request) {
        Ok(result) => CommandOutput::success(render_apply(&result)),
        Err(error) => setup_failure(error),
    }
}

fn execute_uninstall(apply: bool, purge: bool, state_directory: &Path) -> CommandOutput {
    if !cfg!(target_os = "macos") && !cfg!(target_os = "windows") {
        return CommandOutput::error(3, "macOS host uninstall is unsupported on this platform");
    }
    if purge && !apply {
        return CommandOutput::error(2, "uninstall --purge requires --apply");
    }
    #[cfg(target_os = "windows")]
    if apply && !crate::platform::windows::is_elevated() {
        return CommandOutput::error(2, "uninstall --apply must run from an elevated prompt");
    }
    #[cfg(target_os = "macos")]
    let (_, identity) = match setup_user(apply) {
        Ok(user) => user,
        Err(error) => return CommandOutput::error(2, error),
    };
    #[cfg(target_os = "macos")]
    if apply && identity.is_none() {
        return CommandOutput::error(2, "uninstall --apply must run as root through sudo");
    }
    let request = UninstallRequest {
        state_directory: state_directory.to_path_buf(),
        purge,
    };
    let installer = host_installer();
    if !apply {
        return match installer.preview_uninstall(&request) {
            Ok(preview) => CommandOutput::success(render_preview(&preview)),
            Err(error) => setup_failure(error),
        };
    }
    match installer.apply_uninstall(&request) {
        Ok(result) => CommandOutput::success(render_apply(&result)),
        Err(error) => setup_failure(error),
    }
}

fn run_live(client: &Client, command: &CliCommand, json: bool) -> CommandOutput {
    let protocol = match to_protocol(command, client) {
        Ok(command) => command,
        Err(output) => return output,
    };
    match client.request(protocol) {
        Ok(result) => render(result, json, None),
        Err(error) => client_error(error),
    }
}

fn run_offline(cli: &Cli, endpoint: &own_domains_protocol::Endpoint) -> CommandOutput {
    match &cli.command {
        CliCommand::Start => start_daemon(cli, endpoint),
        CliCommand::Stop | CliCommand::Status => CommandOutput::success(if cli.json {
            "{\"runtimeStatus\":\"stopped\"}\n".into()
        } else {
            "stopped\n".into()
        }),
        CliCommand::List => {
            let service = DomainService::new(SettingsStore::new(&cli.state_dir));
            match service.list() {
                Ok(settings) => render(
                    CommandResult::Settings(settings),
                    cli.json,
                    Some("runtime stopped"),
                ),
                Err(error) => CommandOutput::error(4, error),
            }
        }
        CliCommand::Doctor => CommandOutput::success(if cli.json {
            "{\"runtimeStatus\":\"stopped\",\"diagnostics\":[]}\n".into()
        } else {
            "runtime stopped; no active services\n".into()
        }),
        mutation => {
            let _lock = match ProcessLock::acquire(&cli.state_dir) {
                Ok(lock) => lock,
                Err(error) => return CommandOutput::error(4, error),
            };
            let state = DaemonState::new(
                DomainService::new(SettingsStore::new(&cli.state_dir)),
                Arc::new(UnavailableRuntimeBackend),
            );
            let command = match to_offline_protocol(mutation, &state) {
                Ok(command) => command,
                Err(output) => return output,
            };
            let response = state.handle(own_domains_protocol::RequestEnvelope {
                protocol_version: own_domains_protocol::PROTOCOL_VERSION,
                request_id: "offline".into(),
                command,
            });
            match response.result {
                Ok(result) => render(result, cli.json, Some("saved; runtime stopped")),
                Err(error) => protocol_error(error),
            }
        }
    }
}

fn to_protocol(command: &CliCommand, client: &Client) -> Result<Command, CommandOutput> {
    match command {
        CliCommand::Start => Ok(Command::Start),
        CliCommand::Stop => Ok(Command::Stop),
        CliCommand::Status => Ok(Command::Snapshot),
        CliCommand::List => Ok(Command::List),
        CliCommand::Doctor => Ok(Command::Doctor),
        CliCommand::Add {
            hostname,
            port,
            revision,
            https,
        } => Ok(Command::Create {
            input: MappingInput {
                id: None,
                hostname: hostname.clone(),
                upstream_port: *port,
                tls: *https,
                enabled: true,
            },
            expected_revision: *revision,
        }),
        other => {
            let settings = match client.request(Command::List) {
                Ok(CommandResult::Settings(settings)) => settings,
                Ok(_) => {
                    return Err(CommandOutput::error(
                        4,
                        "daemon returned unexpected list result",
                    ));
                }
                Err(error) => return Err(client_error(error)),
            };
            mutation_for_existing(other, &settings)
        }
    }
}

fn to_offline_protocol(
    command: &CliCommand,
    state: &DaemonState,
) -> Result<Command, CommandOutput> {
    if let CliCommand::Add {
        hostname,
        port,
        revision,
        https,
    } = command
    {
        return Ok(Command::Create {
            input: MappingInput {
                id: None,
                hostname: hostname.clone(),
                upstream_port: *port,
                tls: *https,
                enabled: true,
            },
            expected_revision: *revision,
        });
    }
    let settings = state.snapshot().map_err(protocol_error)?.settings;
    mutation_for_existing(command, &settings)
}

fn mutation_for_existing(
    command: &CliCommand,
    settings: &own_domains_core::Settings,
) -> Result<Command, CommandOutput> {
    let (hostname, revision) = match command {
        CliCommand::Edit {
            hostname, revision, ..
        }
        | CliCommand::Enable { hostname, revision }
        | CliCommand::Disable { hostname, revision }
        | CliCommand::Remove { hostname, revision } => (hostname, *revision),
        _ => return Err(CommandOutput::error(4, "not a mutation")),
    };
    let mapping = settings
        .mappings
        .iter()
        .find(|item| item.hostname == *hostname || item.id == *hostname)
        .ok_or_else(|| CommandOutput::error(2, format!("domain mapping not found: {hostname}")))?;
    match command {
        CliCommand::Edit { port, https, .. } => Ok(Command::Update {
            id: mapping.id.clone(),
            patch: MappingPatch {
                hostname: None,
                upstream_port: *port,
                tls: *https,
                enabled: None,
            },
            expected_revision: revision,
        }),
        CliCommand::Enable { .. } => Ok(Command::Update {
            id: mapping.id.clone(),
            patch: MappingPatch {
                enabled: Some(true),
                ..Default::default()
            },
            expected_revision: revision,
        }),
        CliCommand::Disable { .. } => Ok(Command::Update {
            id: mapping.id.clone(),
            patch: MappingPatch {
                enabled: Some(false),
                ..Default::default()
            },
            expected_revision: revision,
        }),
        CliCommand::Remove { .. } => Ok(Command::Remove {
            id: mapping.id.clone(),
            expected_revision: revision,
        }),
        _ => unreachable!(),
    }
}

fn start_daemon(cli: &Cli, endpoint: &own_domains_protocol::Endpoint) -> CommandOutput {
    let executable = std::env::var_os("OWN_DOMAINS_DAEMON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("own-domains-daemon"));
    match connect_or_launch(
        &cli.state_dir,
        endpoint,
        &executable,
        Duration::from_secs(30),
    ) {
        Ok(client) => match client.request(Command::Start) {
            Ok(result) => render(result, cli.json, None),
            Err(error) => client_error(error),
        },
        Err(LifecycleError::Client(error)) => client_error(error),
        Err(error) => CommandOutput::error(4, error),
    }
}

fn render(result: CommandResult, json: bool, note: Option<&str>) -> CommandOutput {
    if let CommandResult::Mutation(mutation) = &result
        && let Some(apply) = &mutation.apply
        && apply.status == ApplyStatus::Error
    {
        let stdout = if json {
            match serde_json::to_string(&result) {
                Ok(value) => format!("{value}\n"),
                Err(error) => return CommandOutput::error(4, error),
            }
        } else {
            format!(
                "revision {}: saved; runtime degraded\n",
                mutation.settings.revision
            )
        };
        return CommandOutput {
            exit_code: 5,
            stdout,
            stderr: format!(
                "{}\n",
                apply
                    .message
                    .as_deref()
                    .unwrap_or("runtime failed to apply the saved configuration")
            ),
        };
    }
    if json {
        return match serde_json::to_string(&result) {
            Ok(value) => CommandOutput::success(format!("{value}\n")),
            Err(error) => CommandOutput::error(4, error),
        };
    }
    let text = match result {
        CommandResult::Settings(settings) => {
            if let Some(note) = note {
                format!("revision {}: {note}", settings.revision)
            } else {
                settings
                    .mappings
                    .iter()
                    .map(|item| format!("{} -> {}", item.hostname, item.upstream_port))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        CommandResult::Mutation(result) => {
            if let Some(note) = note {
                format!("revision {}: {note}", result.settings.revision)
            } else if let Some(apply) = result.apply {
                match apply.status {
                    ApplyStatus::Applied => {
                        format!("revision {}: applied", result.settings.revision)
                    }
                    ApplyStatus::Pending => {
                        format!(
                            "revision {}: saved; apply pending",
                            result.settings.revision
                        )
                    }
                    ApplyStatus::Error => unreachable!("error apply results return above"),
                }
            } else {
                format!("revision {}", result.settings.revision)
            }
        }
        CommandResult::Snapshot(snapshot) => format!(
            "{:?} (revision {})",
            snapshot.runtime_status, snapshot.settings.revision
        )
        .to_lowercase(),
        CommandResult::Runtime(status) => format!("{status:?}").to_lowercase(),
        CommandResult::Diagnostics(items) => {
            if items.is_empty() {
                "no diagnostics".into()
            } else {
                items
                    .into_iter()
                    .map(|item| item.message)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        other => format!("{other:?}"),
    };
    CommandOutput::success(format!("{text}\n"))
}

fn client_error(error: ClientError) -> CommandOutput {
    match error {
        ClientError::Protocol(error) => protocol_error(error),
        ClientError::IncompatibleVersion { .. } => CommandOutput::error(4, error),
        _ => CommandOutput::error(4, error),
    }
}
fn protocol_error(error: ProtocolError) -> CommandOutput {
    let code = match error {
        ProtocolError::Validation { .. }
        | ProtocolError::NotFound { .. }
        | ProtocolError::DuplicateHostname { .. } => 2,
        ProtocolError::RevisionConflict { .. } => 3,
        ProtocolError::RuntimeUnavailable { .. } | ProtocolError::RuntimeFailure { .. } => 5,
        _ => 4,
    };
    CommandOutput::error(code, format!("{error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_failure_preserves_the_input_exit_code() {
        let output = setup_failure(SetupFailure::new("invalid input").with_exit_code(2));
        assert_eq!(output.exit_code, 2);
        assert_eq!(output.stderr, "invalid input\n");
    }

    #[test]
    fn setup_failure_reports_unresolved_items_with_the_same_code() {
        let mut error = SetupFailure::new("host setup failed").with_exit_code(4);
        error.unresolved.push("/etc/pf.conf".into());
        let output = setup_failure(error);
        assert_eq!(output.exit_code, 4);
        assert_eq!(
            output.stderr,
            "host setup failed; unresolved: /etc/pf.conf\n"
        );
    }
}
