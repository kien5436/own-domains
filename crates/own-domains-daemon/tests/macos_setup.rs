use own_domains_daemon::platform::macos::{
    HostCommand, HostCommandOutput, HostFileKind, MacOsArchitecture, MacOsSetupOptions, SetupFile,
    SetupHost, SidecarManifest, SidecarSpec, SystemSetupHost, apply_setup, apply_uninstall,
    build_host_setup_plan, preview_setup, preview_uninstall,
};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

const TEST_CA_PEM: &[u8] =
    b"-----BEGIN CERTIFICATE-----\nUFVCTElDIGNlcnQ=\n-----END CERTIFICATE-----\n";

#[derive(Clone, Debug, Eq, PartialEq)]
struct HostState {
    files: BTreeMap<PathBuf, SetupFile>,
    trusted: bool,
    pf_enabled: bool,
    active_anchor: Option<Vec<u8>>,
}

struct FakeHost {
    state: Mutex<HostState>,
    operation: Mutex<usize>,
    fail_at: Option<usize>,
}

impl FakeHost {
    fn new(state: HostState, fail_at: Option<usize>) -> Self {
        Self {
            state: Mutex::new(state),
            operation: Mutex::new(0),
            fail_at,
        }
    }
    fn step(&self, label: &str) -> Result<(), String> {
        let mut operation = self.operation.lock().unwrap();
        *operation += 1;
        if self.fail_at == Some(*operation) {
            Err(format!("injected failure after {label}"))
        } else {
            Ok(())
        }
    }
    fn snapshot(&self) -> HostState {
        self.state.lock().unwrap().clone()
    }
}

impl SetupHost for FakeHost {
    fn inspect_file(&self, path: &Path) -> Result<Option<SetupFile>, String> {
        Ok(self.state.lock().unwrap().files.get(path).cloned())
    }
    fn atomic_write(&self, path: &Path, file: &SetupFile) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(path.to_path_buf(), file.clone());
        self.step("artifact write")
    }
    fn write_backup(&self, path: &Path, file: &SetupFile) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(path.to_path_buf(), file.clone());
        Ok(())
    }
    fn remove_file(&self, path: &Path) -> Result<(), String> {
        self.state.lock().unwrap().files.remove(path);
        Ok(())
    }
    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .files
            .retain(|candidate, _| !candidate.starts_with(path));
        Ok(())
    }
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        let program = command.program.to_string_lossy();
        if program == "/usr/bin/security" {
            let action = command.args.first().map(String::as_str);
            if action == Some("delete-certificate") {
                assert!(command.args.iter().any(|arg| arg == "-Z"));
                assert!(command.args.iter().any(|arg| arg == "-t"));
                assert!(
                    command
                        .args
                        .iter()
                        .any(|arg| arg == "/Library/Keychains/System.keychain")
                );
                self.state.lock().unwrap().trusted = false;
            } else if action == Some("remove-trusted-cert") {
                let certificate = command.args.get(2).map(PathBuf::from);
                if certificate
                    .as_ref()
                    .is_some_and(|path| !self.state.lock().unwrap().files.contains_key(path))
                {
                    return Err("trusted certificate is missing".into());
                }
                self.state.lock().unwrap().trusted = false;
            } else {
                self.state.lock().unwrap().trusted = action == Some("add-trusted-cert");
            }
            self.step("trust operation")?;
        } else if program == "/sbin/pfctl" && command.args.first().is_some_and(|arg| arg == "-nf") {
            self.step("PF validation")?;
        } else if program == "/sbin/pfctl" && command.args.iter().any(|arg| arg == "-f") {
            let anchor = self
                .state
                .lock()
                .unwrap()
                .files
                .get(Path::new("/etc/pf.anchors/dev.owndomains"))
                .map(|file| file.content.clone());
            self.state.lock().unwrap().active_anchor = anchor;
            self.step("PF load")?;
        } else if program == "/sbin/pfctl" && command.args.first().is_some_and(|arg| arg == "-E") {
            self.state.lock().unwrap().pf_enabled = true;
            self.step("PF enable")?;
        } else if program == "/sbin/pfctl" && command.args.first().is_some_and(|arg| arg == "-d") {
            self.state.lock().unwrap().pf_enabled = false;
        } else if program == "/sbin/pfctl" && command.args.iter().any(|arg| arg == "-F") {
            self.state.lock().unwrap().active_anchor = None;
        }
        Ok(HostCommandOutput {
            status: 0,
            stdout: vec![],
            stderr: vec![],
        })
    }
    fn certificate_trusted(&self, _certificate: &Path) -> Result<bool, String> {
        Ok(self.state.lock().unwrap().trusted)
    }
    fn pf_enabled(&self) -> Result<bool, String> {
        Ok(self.state.lock().unwrap().pf_enabled)
    }
    fn final_probe(&self) -> Result<(), String> {
        self.step("final probe")
    }
}

fn setup_options(address: Ipv4Addr) -> MacOsSetupOptions {
    MacOsSetupOptions {
        user_name: "kien".into(),
        user_id: 501,
        home_directory: "/Users/kien".into(),
        state_directory: "/Users/kien/Library/Application Support/OwnDomains".into(),
        resource_directory: "/Applications/Own Domains.app/Contents/Resources".into(),
        backup_directory: "/Users/kien/Library/Application Support/OwnDomains/backups/setup-1"
            .into(),
        lan_address: address,
        interface: "en0".into(),
        interface_addresses: vec![address],
        architecture: MacOsArchitecture::Arm64,
        sidecars: SidecarManifest {
            schema_version: 1,
            coredns: SidecarSpec {
                version: "1.14.7".into(),
                sha256: "a".repeat(64),
            },
            caddy: SidecarSpec {
                version: "2.11.7".into(),
                sha256: "b".repeat(64),
            },
        },
    }
}

fn initial_state() -> HostState {
    HostState {
        files: BTreeMap::from([(
            PathBuf::from("/etc/pf.conf"),
            SetupFile {
                content: b"set skip on lo0\n".to_vec(),
                mode: 0o644,
                owner_id: 0,
                kind: HostFileKind::Regular,
            },
        )]),
        trusted: false,
        pf_enabled: false,
        active_anchor: None,
    }
}

#[test]
fn every_setup_failure_rolls_back_files_trust_and_pf_state() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    for fail_at in 1..=9 {
        let mut before = initial_state();
        before.files.insert(
            plan.paths.ca_certificate.clone(),
            SetupFile {
                content: TEST_CA_PEM.to_vec(),
                mode: 0o600,
                owner_id: 501,
                kind: HostFileKind::Regular,
            },
        );
        let host = FakeHost::new(before.clone(), Some(fail_at));
        let error = apply_setup(&plan, &host).unwrap_err();

        assert!(error.unresolved.is_empty(), "failure {fail_at}: {error:?}");
        assert_eq!(host.snapshot(), before, "failure {fail_at}");
    }
}

#[test]
fn setup_is_idempotent_and_owned_updates_create_backups() {
    let first = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(initial_state(), None);
    let applied = apply_setup(&first, &host).unwrap();
    assert_eq!(applied.written.len(), 4);
    assert!(
        preview_setup(&first, &host)
            .unwrap()
            .actions
            .iter()
            .all(|action| action.kind == "unchanged")
    );

    let second = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 21))).unwrap();
    let updated = apply_setup(&second, &host).unwrap();
    assert!(
        updated
            .backups
            .iter()
            .any(|path| path.ends_with("etc/pf.anchors/dev.owndomains"))
    );
}

#[test]
fn setup_rejects_unowned_conflicts_and_corrupt_manifests() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let mut conflict = initial_state();
    conflict.files.insert(
        PathBuf::from("/etc/resolver/own"),
        SetupFile {
            content: b"foreign\n".to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    assert!(
        preview_setup(&plan, &FakeHost::new(conflict, None))
            .unwrap_err()
            .to_string()
            .contains("unowned")
    );

    let mut corrupt = initial_state();
    corrupt.files.insert(
        plan.paths.install_manifest.clone(),
        SetupFile {
            content: b"not json".to_vec(),
            mode: 0o600,
            owner_id: 501,
            kind: HostFileKind::Regular,
        },
    );
    assert!(
        preview_setup(&plan, &FakeHost::new(corrupt, None))
            .unwrap_err()
            .to_string()
            .contains("manifest")
    );
}

#[test]
fn uninstall_restores_host_files_and_preserves_user_state_without_purge() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(initial_state(), None);
    apply_setup(&plan, &host).unwrap();
    let settings = plan.paths.state_directory.join("settings.json");
    let ca = plan.paths.ca_certificate.clone();
    host.state.lock().unwrap().files.insert(
        settings.clone(),
        SetupFile {
            content: b"settings".to_vec(),
            mode: 0o600,
            owner_id: 501,
            kind: HostFileKind::Regular,
        },
    );
    host.state.lock().unwrap().files.insert(
        ca.clone(),
        SetupFile {
            content: TEST_CA_PEM.to_vec(),
            mode: 0o600,
            owner_id: 501,
            kind: HostFileKind::Regular,
        },
    );

    let preview = preview_uninstall(&plan.paths, false, &host).unwrap();
    assert!(
        preview
            .actions
            .iter()
            .any(|action| action.kind == "preserve")
    );
    apply_uninstall(&plan.paths, false, &host).unwrap();
    let state = host.snapshot();
    assert_eq!(
        state.files.get(Path::new("/etc/pf.conf")).unwrap().content,
        b"set skip on lo0\n"
    );
    assert!(!state.files.contains_key(Path::new("/etc/resolver/own")));
    assert!(state.files.contains_key(&settings));
    assert!(state.files.contains_key(&ca));
}

#[test]
fn system_setup_host_writes_owned_files_atomically_and_removes_trees() {
    let directory = std::env::temp_dir().join(format!(
        "own-domains-system-setup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("nested").join("owned.conf");
    let file = SetupFile {
        content: b"nameserver 127.0.0.1\n".to_vec(),
        mode: 0o600,
        owner_id: unsafe { libc::geteuid() },
        kind: HostFileKind::Regular,
    };
    let host = SystemSetupHost;

    host.atomic_write(&path, &file).unwrap();
    let inspected = host.inspect_file(&path).unwrap().unwrap();
    assert_eq!(inspected, file);
    assert_eq!(
        std::fs::symlink_metadata(&path).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(
        host.inspect_file(&directory).unwrap().unwrap().kind,
        HostFileKind::Other
    );

    host.remove_file(&path).unwrap();
    assert!(host.inspect_file(&path).unwrap().is_none());
    host.remove_file(&path).unwrap();

    host.remove_tree(&directory).unwrap();
    assert!(host.inspect_file(&directory).unwrap().is_none());
    host.remove_tree(&directory).unwrap();
}

#[test]
fn purge_removes_trust_before_removing_the_ca_state() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(initial_state(), None);
    apply_setup(&plan, &host).unwrap();
    host.state.lock().unwrap().files.insert(
        plan.paths.ca_certificate.clone(),
        SetupFile {
            content: TEST_CA_PEM.to_vec(),
            mode: 0o600,
            owner_id: 501,
            kind: HostFileKind::Regular,
        },
    );
    assert!(host.snapshot().trusted);

    apply_uninstall(&plan.paths, true, &host).unwrap();

    assert!(!host.snapshot().trusted);
    assert!(
        !host
            .snapshot()
            .files
            .contains_key(&plan.paths.ca_certificate)
    );
}
