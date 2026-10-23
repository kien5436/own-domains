use crate::privilege::{UserRecord, setup_user, user_by_uid};
use clap::{Parser, Subcommand};
use own_domains_client::{Client, ClientError, LifecycleError, connect_or_launch, stop_and_wait};
use own_domains_core::{ApplyStatus, DomainService, MappingInput, MappingPatch, SettingsStore};
use own_domains_daemon::{
    DaemonState, ProcessLock, UnavailableRuntimeBackend,
    platform::macos::{
        CommandRunner, HostCommand, HostFileKind, HostFileSystem, MacOsArchitecture,
        MacOsSetupOptions, RuntimePaths, SidecarManifest, SystemCommandRunner,
        SystemHostFileSystem, SystemSetupHost, apply_setup, apply_uninstall, build_host_setup_plan,
        preview_setup, preview_uninstall,
    },
};
use own_domains_protocol::{Command, CommandResult, ProtocolError};
use std::{
    net::Ipv4Addr,
    path::{Component, Path, PathBuf},
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
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(|| cli.state_dir.join("daemon.sock"));
    let client_timeout = if matches!(cli.command, CliCommand::Start) {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(5)
    };
    match Client::with_timeout(&socket, client_timeout) {
        Ok(client) if matches!(cli.command, CliCommand::Stop) => {
            match stop_and_wait(&client, &socket, Duration::from_secs(5)) {
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
        Err(ClientError::DaemonUnavailable { .. }) => run_offline(&cli, &socket),
        Err(error) => client_error(error),
    }
}

fn normalize_absolute(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("path must be absolute: {}", path.display()));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!("path escapes its root: {}", path.display()));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

fn target_triple(architecture: MacOsArchitecture) -> &'static str {
    match architecture {
        MacOsArchitecture::Arm64 => "aarch64-apple-darwin",
        MacOsArchitecture::X86_64 => "x86_64-apple-darwin",
    }
}

fn current_architecture() -> MacOsArchitecture {
    if cfg!(target_arch = "aarch64") {
        MacOsArchitecture::Arm64
    } else {
        MacOsArchitecture::X86_64
    }
}

fn packaged_resource_directory(executable: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("OWN_DOMAINS_RESOURCES") {
        let path = normalize_absolute(Path::new(&path))?;
        if path.join("sidecars").is_dir() {
            return Ok(path);
        }
        return Err(format!(
            "OWN_DOMAINS_RESOURCES does not contain sidecars: {}",
            path.display()
        ));
    }
    let executable = executable
        .canonicalize()
        .map_err(|error| format!("unable to resolve the CLI executable: {error}"))?;
    let parent = executable
        .parent()
        .ok_or_else(|| "the CLI executable has no parent directory".to_string())?;
    for candidate in [
        parent.to_path_buf(),
        parent.join("Resources"),
        parent.join("../Resources"),
    ] {
        if let Ok(candidate) = normalize_absolute(&candidate)
            && candidate.join("sidecars").is_dir()
        {
            return Ok(candidate);
        }
    }
    Err("packaged sidecars are missing; install the release package".into())
}

fn installed_resource_directory(executable: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("OWN_DOMAINS_RESOURCES") {
        return normalize_absolute(Path::new(&path));
    }
    let executable = executable
        .canonicalize()
        .map_err(|error| format!("unable to resolve the CLI executable: {error}"))?;
    let parent = executable
        .parent()
        .ok_or_else(|| "the CLI executable has no parent directory".to_string())?;
    for candidate in [parent.join("../Resources"), parent.to_path_buf()] {
        if let Ok(candidate) = normalize_absolute(&candidate)
            && candidate.is_dir()
        {
            return Ok(candidate);
        }
    }
    Err("unable to resolve the installed resource directory".into())
}

fn manifest_for_architecture(
    resource_directory: &Path,
    architecture: MacOsArchitecture,
) -> Result<SidecarManifest, String> {
    let target = target_triple(architecture);
    let sidecars = resource_directory.join("sidecars");
    let target_manifest = sidecars.join(format!("manifest-{target}.json"));
    let manifest_path = if target_manifest.is_file() {
        target_manifest
    } else {
        sidecars.join("manifest.json")
    };
    let bytes = std::fs::read(&manifest_path)
        .map_err(|error| format!("unable to read {}: {error}", manifest_path.display()))?;
    serde_json::from_slice::<SidecarManifest>(&bytes).map_err(|error| {
        format!(
            "invalid sidecar manifest {}: {error}",
            manifest_path.display()
        )
    })
}

fn verify_sidecar_binaries(paths: &RuntimePaths, sidecars: &SidecarManifest) -> Result<(), String> {
    let files = SystemHostFileSystem;
    for (name, path, expected) in [
        ("CoreDNS", &paths.coredns_binary, &sidecars.coredns.sha256),
        ("Caddy", &paths.caddy_binary, &sidecars.caddy.sha256),
    ] {
        let metadata = files
            .metadata(path)?
            .ok_or_else(|| format!("{name} sidecar is missing: {}", path.display()))?;
        if metadata.kind != HostFileKind::Regular || metadata.mode & 0o111 == 0 {
            return Err(format!(
                "{name} sidecar is not executable: {}",
                path.display()
            ));
        }
        let digest = files.sha256(path, 256 * 1024 * 1024)?;
        if digest != expected.to_ascii_lowercase() {
            return Err(format!(
                "{name} sidecar digest does not match the packaged manifest: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn resolve_state_directory(state_directory: &Path, home: &Path) -> Result<PathBuf, String> {
    let path = if state_directory.is_absolute() {
        state_directory.to_path_buf()
    } else {
        home.join(state_directory)
    };
    let path = normalize_absolute(&path)?;
    if !path.starts_with(home) {
        return Err("state directory must remain inside the user's home".into());
    }
    Ok(path)
}

fn interface_addresses(interface: &str) -> Result<Vec<Ipv4Addr>, String> {
    let output = SystemCommandRunner.run(&HostCommand {
        program: "/sbin/ifconfig".into(),
        args: vec![interface.to_string()],
        timeout: Duration::from_secs(2),
    })?;
    if output.status != 0 {
        return Err(format!(
            "ifconfig failed for {interface}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let mut addresses = Vec::new();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        while let Some(field) = fields.next() {
            if field == "inet"
                && let Some(address) = fields.next()
                && let Ok(address) = address.parse::<Ipv4Addr>()
            {
                addresses.push(address);
            }
        }
    }
    if addresses.is_empty() {
        return Err(format!("interface {interface} has no IPv4 address"));
    }
    Ok(addresses)
}

fn setup_options(
    state_directory: &Path,
    lan_address: &str,
    interface: &str,
) -> Result<(MacOsSetupOptions, RuntimePaths), String> {
    let (user, _) = setup_user(false)?;
    let architecture = current_architecture();
    let resource_directory =
        packaged_resource_directory(&std::env::current_exe().map_err(|error| error.to_string())?)?;
    let state_directory = resolve_state_directory(state_directory, &user.home)?;
    let backup_directory = state_directory.join("backups").join("setup");
    let lan_address = lan_address
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("invalid LAN IPv4 address: {lan_address}"))?;
    let addresses = interface_addresses(interface)?;
    let sidecars = manifest_for_architecture(&resource_directory, architecture)?;
    let paths = RuntimePaths::new(
        state_directory.clone(),
        resource_directory.clone(),
        backup_directory.clone(),
        architecture,
    )
    .map_err(|error| error.to_string())?;
    verify_sidecar_binaries(&paths, &sidecars)?;
    let options = MacOsSetupOptions {
        user_name: user.name.clone(),
        user_id: user.user_id,
        home_directory: user.home.clone(),
        state_directory,
        resource_directory,
        backup_directory,
        lan_address,
        interface: interface.to_string(),
        interface_addresses: addresses,
        architecture,
        sidecars,
    };
    Ok((options, paths))
}

fn chown_path(path: &Path, user_id: u32, group_id: u32) -> Result<(), String> {
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| "path contains a NUL byte".to_string())?;
    if unsafe { libc::chown(path.as_ptr(), user_id, group_id) } != 0 {
        return Err(format!(
            "unable to set ownership: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn create_owned_directory(path: &Path, user: &UserRecord) -> Result<(), String> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("unable to create {}: {error}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
            chown_path(path, user.user_id, user.group_id)?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata =
            std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if metadata.uid() != user.user_id {
            return Err(format!(
                "directory must be owned by {}: {}",
                user.name,
                path.display()
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaddyCaState {
    Missing,
    Incomplete,
    Ready,
}

fn nonempty_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

fn caddy_ca_state(certificate: &Path, key: &Path) -> CaddyCaState {
    match (nonempty_file(certificate), nonempty_file(key)) {
        (true, true) => CaddyCaState::Ready,
        (false, false) => CaddyCaState::Missing,
        _ => CaddyCaState::Incomplete,
    }
}

fn ensure_caddy_ca(paths: &RuntimePaths, user: &UserRecord) -> Result<(), String> {
    let ca_key = paths.ca_certificate.with_file_name("root.key");
    match caddy_ca_state(&paths.ca_certificate, &ca_key) {
        CaddyCaState::Ready => return Ok(()),
        CaddyCaState::Incomplete => {
            let ca_directory = paths.ca_certificate.parent().ok_or_else(|| {
                format!(
                    "cannot determine Caddy CA directory from {}",
                    paths.ca_certificate.display()
                )
            })?;
            std::fs::remove_dir_all(ca_directory)
                .map_err(|error| format!("unable to remove {}: {error}", ca_directory.display()))?;
        }
        CaddyCaState::Missing => {}
    }
    create_owned_directory(&paths.runtime_directory, user)?;
    let admin_socket = paths.runtime_directory.join("ca-admin.sock");
    let _ = std::fs::remove_file(&admin_socket);
    let config_path = paths.runtime_directory.join("ca-generation.json");
    let config_home = paths.state_directory.join("caddy-config");
    let config = serde_json::json!({
        "admin": {"listen": format!("unix/{}", admin_socket.to_string_lossy())},
        "storage": {"module": "file_system", "root": paths.caddy_data_directory.to_string_lossy()},
        "apps": {
            "pki": {"certificate_authorities": {"local": {"install_trust": false}}},
            "tls": {"automation": {"policies": [{"subjects": ["setup.own"], "issuers": [{"module": "internal"}]}]}}
        }
    });
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("unable to write {}: {error}", config_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        chown_path(&config_path, user.user_id, user.group_id)?;
    }
    let temporary_directory = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let mut command = std::process::Command::new(&paths.caddy_binary);
    command
        .arg("run")
        .arg("--config")
        .arg(&config_path)
        .env_clear()
        .env("HOME", &user.home)
        .env("TMPDIR", temporary_directory)
        .env("XDG_DATA_HOME", &paths.caddy_data_directory)
        .env("XDG_CONFIG_HOME", &config_home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.uid(user.user_id).gid(user.group_id);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("unable to start Caddy for CA generation: {error}"))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if caddy_ca_state(&paths.ca_certificate, &ca_key) == CaddyCaState::Ready {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&admin_socket);
            let _ = std::fs::remove_file(&config_path);
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            let _ = std::fs::remove_file(&admin_socket);
            let _ = std::fs::remove_file(&config_path);
            return Err(format!(
                "Caddy exited with {status} before generating the root certificate and key"
            ));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&admin_socket);
            let _ = std::fs::remove_file(&config_path);
            return Err("timed out while generating the Caddy root certificate and key".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ensure_state_directory(paths: &RuntimePaths, user: &UserRecord) -> Result<(), String> {
    create_owned_directory(&paths.state_directory, user)
}

fn setup_failure(error: own_domains_daemon::platform::macos::SetupFailure) -> CommandOutput {
    if error.unresolved.is_empty() {
        CommandOutput::error(4, error.message)
    } else {
        CommandOutput::error(
            4,
            format!(
                "{}; unresolved: {}",
                error.message,
                error.unresolved.join(", ")
            ),
        )
    }
}

fn persist_network_address(state_directory: &Path, lan_address: &str) -> Result<(), String> {
    let store = SettingsStore::new(state_directory);
    let mut settings = store.load().map_err(|error| error.to_string())?;
    settings.lan_address = lan_address.to_string();
    let revision = settings.revision;
    store
        .save(&settings, Some(revision))
        .map_err(|error| error.to_string())
}

fn execute_setup(
    apply: bool,
    state_directory: &Path,
    lan_address: &str,
    interface: &str,
) -> CommandOutput {
    if !cfg!(target_os = "macos") {
        return CommandOutput::error(3, "macOS host setup is unsupported on this platform");
    }
    let (_, identity) = match setup_user(apply) {
        Ok(user) => user,
        Err(error) => return CommandOutput::error(2, error),
    };
    if apply && identity.is_none() {
        return CommandOutput::error(2, "setup --apply must run as root through sudo");
    }
    let (options, paths) = match setup_options(state_directory, lan_address, interface) {
        Ok(options) => options,
        Err(error) => return CommandOutput::error(4, error),
    };
    let plan = match build_host_setup_plan(&options) {
        Ok(plan) => plan,
        Err(error) => return CommandOutput::error(2, error),
    };
    let host = SystemSetupHost;
    if !apply {
        return match preview_setup(&plan, &host) {
            Ok(preview) => CommandOutput::success(
                preview
                    .actions
                    .iter()
                    .map(|action| format!("{} {}", action.kind, action.path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            ),
            Err(error) => setup_failure(error),
        };
    }
    let user = match user_by_uid(options.user_id) {
        Ok(user) => user,
        Err(error) => return CommandOutput::error(4, error),
    };
    if let Err(error) = ensure_state_directory(&paths, &user) {
        return CommandOutput::error(4, error);
    }
    if let Err(error) = ensure_caddy_ca(&paths, &user) {
        return CommandOutput::error(4, error);
    }
    match apply_setup(&plan, &host) {
        Ok(result) => {
            let settings_path = paths.state_directory.join("settings.json");
            let persisted =
                persist_network_address(&paths.state_directory, &options.lan_address.to_string())
                    .and_then(|()| chown_path(&settings_path, user.user_id, user.group_id));
            if let Err(error) = persisted {
                let rollback = match apply_uninstall(&paths, false, &host) {
                    Ok(_) => "host setup rolled back".to_string(),
                    Err(rollback) => {
                        format!("host setup rollback failed: {}", rollback.message)
                    }
                };
                return CommandOutput::error(
                    4,
                    format!("unable to persist LAN address: {error}; {rollback}"),
                );
            }
            CommandOutput::success(
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
                    + "\n",
            )
        }
        Err(error) => setup_failure(error),
    }
}

fn execute_uninstall(apply: bool, purge: bool, state_directory: &Path) -> CommandOutput {
    if !cfg!(target_os = "macos") {
        return CommandOutput::error(3, "macOS host uninstall is unsupported on this platform");
    }
    if purge && !apply {
        return CommandOutput::error(2, "uninstall --purge requires --apply");
    }
    let (user, identity) = match setup_user(apply) {
        Ok(user) => user,
        Err(error) => return CommandOutput::error(2, error),
    };
    if apply && identity.is_none() {
        return CommandOutput::error(2, "uninstall --apply must run as root through sudo");
    }
    let architecture = current_architecture();
    let resource_directory = match std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|executable| installed_resource_directory(&executable))
    {
        Ok(path) => path,
        Err(error) => return CommandOutput::error(4, error),
    };
    let state_directory = match resolve_state_directory(state_directory, &user.home) {
        Ok(path) => path,
        Err(error) => return CommandOutput::error(2, error),
    };
    let backup_directory = state_directory.join("backups").join("setup");
    let paths = match RuntimePaths::new(
        state_directory,
        resource_directory,
        backup_directory,
        architecture,
    ) {
        Ok(paths) => paths,
        Err(error) => return CommandOutput::error(2, error),
    };
    let host = SystemSetupHost;
    if !apply {
        return match preview_uninstall(&paths, purge, &host) {
            Ok(preview) => CommandOutput::success(
                preview
                    .actions
                    .iter()
                    .map(|action| format!("{} {}", action.kind, action.path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            ),
            Err(error) => setup_failure(error),
        };
    }
    match apply_uninstall(&paths, purge, &host) {
        Ok(result) => CommandOutput::success(
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
                + "\n",
        ),
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

fn run_offline(cli: &Cli, socket: &std::path::Path) -> CommandOutput {
    match &cli.command {
        CliCommand::Start => start_daemon(cli, socket),
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

fn start_daemon(cli: &Cli, socket: &std::path::Path) -> CommandOutput {
    let executable = std::env::var_os("OWN_DOMAINS_DAEMON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("own-domains-daemon"));
    match connect_or_launch(&cli.state_dir, socket, &executable, Duration::from_secs(30)) {
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
    use own_domains_core::{DomainMapping, Settings};

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "own-domains-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn normalizes_absolute_paths_without_allowing_escape() {
        assert_eq!(
            normalize_absolute(Path::new("/Users/example/./state/../state")).unwrap(),
            PathBuf::from("/Users/example/state")
        );
        assert!(normalize_absolute(Path::new("/..")).is_err());
        assert!(normalize_absolute(Path::new("relative")).is_err());
    }

    #[test]
    fn setup_network_update_persists_lan_address_without_resetting_settings() {
        let state = temporary_directory("setup-network");
        let store = SettingsStore::new(&state);
        let mapping = DomainMapping {
            id: "mapping-1".into(),
            hostname: "app.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        };
        let original = Settings {
            revision: 7,
            lan_address: String::new(),
            upstream_dns: vec!["8.8.8.8".into()],
            mappings: vec![mapping.clone()],
            ..Default::default()
        };
        store.save(&original, Some(0)).unwrap();

        persist_network_address(&state, "192.168.1.219").unwrap();

        let updated = store.load().unwrap();
        assert_eq!(updated.lan_address, "192.168.1.219");
        assert_eq!(updated.revision, 7);
        assert_eq!(updated.upstream_dns, original.upstream_dns);
        assert_eq!(updated.mappings, vec![mapping]);
        std::fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn resolves_relative_state_directories_inside_the_user_home() {
        let home = Path::new("/Users/example");
        assert_eq!(
            resolve_state_directory(Path::new(".own-domains"), home).unwrap(),
            PathBuf::from("/Users/example/.own-domains")
        );
        assert!(resolve_state_directory(Path::new("/Users/example2/state"), home).is_err());
        assert!(resolve_state_directory(Path::new("/Users/example/../other"), home).is_err());
    }

    #[test]
    fn prefers_packaged_resources_next_to_the_executable_then_contents_resources() {
        let root = temporary_directory("resources");
        let executable = root.join("app").join("own-domains");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"cli").unwrap();
        let resources = root.join("app").join("Resources");
        std::fs::create_dir_all(resources.join("sidecars")).unwrap();

        assert_eq!(
            packaged_resource_directory(&executable).unwrap(),
            resources.canonicalize().unwrap()
        );

        std::fs::create_dir_all(root.join("app").join("sidecars")).unwrap();
        assert_eq!(
            packaged_resource_directory(&executable).unwrap(),
            root.join("app").canonicalize().unwrap()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selects_the_target_specific_manifest_when_present() {
        let root = temporary_directory("manifest");
        let sidecars = root.join("sidecars");
        std::fs::create_dir_all(&sidecars).unwrap();
        let target = sidecars.join("manifest-aarch64-apple-darwin.json");
        std::fs::write(
            &target,
            br#"{"schemaVersion":1,"coredns":{"version":"1.14.7","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"caddy":{"version":"2.11.7","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}"#,
        )
        .unwrap();

        let manifest = manifest_for_architecture(&root, MacOsArchitecture::Arm64).unwrap();
        assert_eq!(manifest.coredns.version, "1.14.7");
        assert_eq!(manifest.caddy.version, "2.11.7");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn caddy_ca_requires_both_certificate_and_nonempty_key() {
        let root = temporary_directory("caddy-ca");
        let certificate = root.join("root.crt");
        let key = root.join("root.key");

        assert_eq!(caddy_ca_state(&certificate, &key), CaddyCaState::Missing);

        std::fs::write(&certificate, b"certificate").unwrap();
        assert_eq!(caddy_ca_state(&certificate, &key), CaddyCaState::Incomplete);

        std::fs::write(&key, b"key").unwrap();
        assert_eq!(caddy_ca_state(&certificate, &key), CaddyCaState::Ready);

        std::fs::write(&key, b"").unwrap();
        assert_eq!(caddy_ca_state(&certificate, &key), CaddyCaState::Incomplete);

        std::fs::write(&certificate, b"").unwrap();
        assert_eq!(caddy_ca_state(&certificate, &key), CaddyCaState::Missing);

        std::fs::remove_dir_all(root).unwrap();
    }
}
