use crate::privilege::{UserRecord, setup_user, user_by_uid};
use own_domains_core::SettingsStore;
use own_domains_daemon::platform::macos::{
    CommandRunner, HostCommand, HostFileKind, HostFileSystem, MacOsArchitecture, MacOsSetupOptions,
    RuntimePaths, SidecarManifest, SystemCommandRunner, SystemHostFileSystem, SystemSetupHost,
    apply_setup as apply_macos_setup, apply_uninstall as apply_macos_uninstall,
    build_host_setup_plan, preview_setup as preview_macos_setup,
    preview_uninstall as preview_macos_uninstall,
};
use own_domains_daemon::platform::{
    HostInstaller, SetupApplyResult, SetupFailure, SetupPreview, SetupRequest, UninstallRequest,
};
use std::{
    net::Ipv4Addr,
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[allow(dead_code)]
pub(crate) struct MacOsHostInstaller;

impl HostInstaller for MacOsHostInstaller {
    fn preview_setup(&self, request: &SetupRequest) -> Result<SetupPreview, SetupFailure> {
        let (options, _) = setup_options(
            &request.state_directory,
            &request.lan_address,
            &request.interface,
        )
        .map_err(SetupFailure::new)?;
        let plan = build_host_setup_plan(&options)
            .map_err(|error| SetupFailure::new(error.to_string()).with_exit_code(2))?;
        preview_macos_setup(&plan, &SystemSetupHost)
    }

    fn apply_setup(&self, request: &SetupRequest) -> Result<SetupApplyResult, SetupFailure> {
        let (options, paths) = setup_options(
            &request.state_directory,
            &request.lan_address,
            &request.interface,
        )
        .map_err(SetupFailure::new)?;
        let plan = build_host_setup_plan(&options)
            .map_err(|error| SetupFailure::new(error.to_string()).with_exit_code(2))?;
        let user = user_by_uid(options.user_id).map_err(SetupFailure::new)?;
        ensure_state_directory(&paths, &user).map_err(SetupFailure::new)?;
        ensure_caddy_ca(&paths, &user).map_err(SetupFailure::new)?;
        let result = apply_macos_setup(&plan, &SystemSetupHost)?;
        let settings_path = paths.state_directory.join("settings.json");
        let persisted =
            persist_network_address(&paths.state_directory, &options.lan_address.to_string())
                .and_then(|()| chown_path(&settings_path, user.user_id, user.group_id));
        if let Err(error) = persisted {
            let rollback = match apply_macos_uninstall(&paths, false, &SystemSetupHost) {
                Ok(_) => "host setup rolled back".to_string(),
                Err(rollback) => {
                    format!("host setup rollback failed: {}", rollback.message)
                }
            };
            return Err(SetupFailure::new(format!(
                "unable to persist LAN address: {error}; {rollback}"
            )));
        }
        Ok(result)
    }

    fn preview_uninstall(&self, request: &UninstallRequest) -> Result<SetupPreview, SetupFailure> {
        let paths = uninstall_paths(&request.state_directory)?;
        preview_macos_uninstall(&paths, request.purge, &SystemSetupHost)
    }

    fn apply_uninstall(
        &self,
        request: &UninstallRequest,
    ) -> Result<SetupApplyResult, SetupFailure> {
        let paths = uninstall_paths(&request.state_directory)?;
        apply_macos_uninstall(&paths, request.purge, &SystemSetupHost)
    }
}

fn uninstall_paths(state_directory: &Path) -> Result<RuntimePaths, SetupFailure> {
    let (user, _) =
        setup_user(false).map_err(|error| SetupFailure::new(error).with_exit_code(2))?;
    let architecture = current_architecture();
    let resource_directory = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|executable| installed_resource_directory(&executable))
        .map_err(SetupFailure::new)?;
    let state_directory = resolve_state_directory(state_directory, &user.home)
        .map_err(|error| SetupFailure::new(error).with_exit_code(2))?;
    let backup_directory = state_directory.join("backups").join("setup");
    RuntimePaths::new(
        state_directory,
        resource_directory,
        backup_directory,
        architecture,
    )
    .map_err(|error| SetupFailure::new(error.to_string()).with_exit_code(2))
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

fn persist_network_address(state_directory: &Path, lan_address: &str) -> Result<(), String> {
    let store = SettingsStore::new(state_directory);
    let mut settings = store.load().map_err(|error| error.to_string())?;
    settings.lan_address = lan_address.to_string();
    let revision = settings.revision;
    store
        .save(&settings, Some(revision))
        .map_err(|error| error.to_string())
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
