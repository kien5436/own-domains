#![cfg(target_os = "macos")]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use own_domains_daemon::platform::macos::{
    HostCommand, HostCommandOutput, HostFileKind, HostSetupPlan, MacOsArchitecture,
    MacOsSetupOptions, ManifestArtifact, NetworkInterface, SetupFile, SetupHost, SidecarManifest,
    SidecarSpec, SystemSetupHost, apply_setup, apply_uninstall, build_host_setup_plan,
    preview_setup, preview_uninstall,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    certificates: BTreeSet<String>,
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
            if action == Some("find-certificate") {
                let listing = self
                    .state
                    .lock()
                    .unwrap()
                    .certificates
                    .iter()
                    .map(|fingerprint| format!("SHA-256 hash: {}\n", fingerprint.to_uppercase()))
                    .collect::<String>();
                self.step("trust inspection")?;
                return Ok(HostCommandOutput {
                    status: 0,
                    stdout: listing.into_bytes(),
                    stderr: vec![],
                });
            }
            if action == Some("delete-certificate") {
                assert!(command.args.iter().any(|arg| arg == "-Z"));
                assert!(!command.args.iter().any(|arg| arg == "-t"));
                assert!(
                    command
                        .args
                        .iter()
                        .any(|arg| arg == "/Library/Keychains/System.keychain")
                );
                let fingerprint = command
                    .args
                    .iter()
                    .position(|arg| arg == "-Z")
                    .and_then(|index| command.args.get(index + 1))
                    .expect("delete-certificate fingerprint");
                let removed = self
                    .state
                    .lock()
                    .unwrap()
                    .certificates
                    .remove(&fingerprint.to_ascii_lowercase());
                if !removed {
                    return Ok(HostCommandOutput {
                        status: 1,
                        stdout: vec![],
                        stderr: b"certificate not found".to_vec(),
                    });
                }
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
                if action == Some("add-trusted-cert") {
                    self.state
                        .lock()
                        .unwrap()
                        .certificates
                        .insert(test_ca_fingerprint());
                }
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
        covered_interfaces: vec![NetworkInterface {
            name: "en0".into(),
            primary_ipv4: Some(address),
        }],
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
        certificates: BTreeSet::new(),
        trusted: false,
        pf_enabled: false,
        active_anchor: None,
    }
}

const LEGACY_ANCHOR: &str =
    "rdr pass on lo0 inet proto udp from any to 192.168.1.21 port 53 -> 192.168.1.21 port 5354\n";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn test_ca_fingerprint() -> String {
    let pem = std::str::from_utf8(TEST_CA_PEM).unwrap();
    let body = pem
        .lines()
        .filter(|line| !line.trim_start().starts_with("-----"))
        .map(str::trim)
        .collect::<String>();
    let der = STANDARD.decode(body.as_bytes()).unwrap();
    format!("{:x}", Sha256::digest(der))
}

fn schema_one_state(plan: &HostSetupPlan) -> HostState {
    let mut state = initial_state();
    state.files.insert(
        PathBuf::from("/etc/resolver/own"),
        SetupFile {
            content: b"nameserver 127.0.0.1\nport 5354\n".to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    state.files.insert(
        PathBuf::from("/etc/pf.anchors/dev.owndomains"),
        SetupFile {
            content: LEGACY_ANCHOR.as_bytes().to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    let pf_block = plan.artifact("/etc/pf.conf").unwrap();
    let artifacts = vec![
        ManifestArtifact {
            path: PathBuf::from("/etc/resolver/own"),
            sha256: digest(b"nameserver 127.0.0.1\nport 5354\n"),
            mode: 0o644,
            owner_id: 0,
            managed_block: false,
        },
        ManifestArtifact {
            path: PathBuf::from("/etc/pf.anchors/dev.owndomains"),
            sha256: digest(LEGACY_ANCHOR.as_bytes()),
            mode: 0o644,
            owner_id: 0,
            managed_block: false,
        },
        ManifestArtifact {
            path: PathBuf::from("/etc/pf.conf"),
            sha256: digest(pf_block.content.as_bytes()),
            mode: 0o644,
            owner_id: 0,
            managed_block: true,
        },
    ];
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "userName": plan.manifest.user_name,
        "userId": plan.manifest.user_id,
        "backupDirectory": plan.manifest.backup_directory,
        "interface": "en0",
        "lanAddress": "192.168.1.21",
        "artifacts": artifacts,
        "sidecars": plan.manifest.sidecars,
    });
    state.files.insert(
        plan.paths.install_manifest.clone(),
        SetupFile {
            content: serde_json::to_vec_pretty(&manifest).unwrap(),
            mode: 0o600,
            owner_id: 501,
            kind: HostFileKind::Regular,
        },
    );
    state
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
fn setup_is_idempotent_and_address_changes_do_not_rewrite_privileged_artifacts() {
    let first = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let second = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 21))).unwrap();
    let anchor = first.artifact("/etc/pf.anchors/dev.owndomains").unwrap();
    assert!(!anchor.content.contains("192.168.1.20"));
    assert_eq!(
        anchor.content,
        second
            .artifact("/etc/pf.anchors/dev.owndomains")
            .unwrap()
            .content
    );
    assert_eq!(first.manifest, second.manifest);

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
    let preview = preview_setup(&second, &host).unwrap();
    assert!(
        preview
            .actions
            .iter()
            .all(|action| action.kind == "unchanged"),
        "an address change must not rewrite the anchor or manifest"
    );
}

#[test]
fn adding_a_present_interface_updates_the_anchor_and_manifest() {
    let first = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(initial_state(), None);
    apply_setup(&first, &host).unwrap();

    let mut options = setup_options(Ipv4Addr::new(192, 168, 1, 20));
    options.covered_interfaces.push(NetworkInterface {
        name: "en1".into(),
        primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
    });
    let second = build_host_setup_plan(&options).unwrap();
    let updated = apply_setup(&second, &host).unwrap();

    assert!(
        updated
            .written
            .iter()
            .any(|path| path == Path::new("/etc/pf.anchors/dev.owndomains"))
    );
    assert!(
        updated
            .backups
            .iter()
            .any(|path| path.ends_with("etc/pf.anchors/dev.owndomains"))
    );
    let anchor = host
        .snapshot()
        .files
        .get(Path::new("/etc/pf.anchors/dev.owndomains"))
        .unwrap()
        .content
        .clone();
    assert!(String::from_utf8(anchor).unwrap().contains("(en1:0)"));
    assert_eq!(second.manifest.interfaces, vec!["en0", "en1"]);
}

#[test]
fn migrates_a_schema_one_manifest_without_the_old_address() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(schema_one_state(&plan), None);

    apply_setup(&plan, &host).unwrap();

    let manifest = host
        .snapshot()
        .files
        .get(&plan.paths.install_manifest)
        .unwrap()
        .content
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["interfaces"], serde_json::json!(["en0"]));
    assert!(value.get("lanAddress").is_none());
    assert!(value.get("interface").is_none());
    let anchor = host
        .snapshot()
        .files
        .get(Path::new("/etc/pf.anchors/dev.owndomains"))
        .unwrap()
        .content
        .clone();
    assert!(String::from_utf8(anchor).unwrap().contains("(en0:0)"));
    let backup = host
        .snapshot()
        .files
        .get(
            &plan
                .manifest
                .backup_directory
                .join("etc/pf.anchors/dev.owndomains"),
        )
        .unwrap()
        .content
        .clone();
    assert_eq!(backup, LEGACY_ANCHOR.as_bytes());
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
fn uninstall_reads_a_schema_one_manifest_and_restores_the_pf_backup() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let mut state = schema_one_state(&plan);
    state
        .files
        .get_mut(Path::new("/etc/pf.conf"))
        .unwrap()
        .content = b"current\n".to_vec();
    state.files.insert(
        plan.manifest.backup_directory.join("etc/pf.conf"),
        SetupFile {
            content: b"original\n".to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    let host = FakeHost::new(state, None);

    let preview = preview_uninstall(&plan.paths, false, &host).unwrap();
    assert!(
        preview
            .actions
            .iter()
            .any(|action| { action.path == Path::new("/etc/pf.conf") && action.kind == "restore" })
    );

    apply_uninstall(&plan.paths, false, &host).unwrap();
    let state = host.snapshot();
    assert_eq!(
        state.files.get(Path::new("/etc/pf.conf")).unwrap().content,
        b"original\n"
    );
    assert!(!state.files.contains_key(Path::new("/etc/resolver/own")));
    assert!(!state.files.contains_key(&plan.paths.install_manifest));
}

#[test]
fn setup_and_uninstall_reject_a_tampered_manifest() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();

    let mut tampered = schema_one_state(&plan);
    let manifest = tampered
        .files
        .get_mut(&plan.paths.install_manifest)
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&manifest.content).unwrap();
    value["artifacts"][0]["path"] = "/etc/shadow".into();
    manifest.content = serde_json::to_vec_pretty(&value).unwrap();
    let error = preview_setup(&plan, &FakeHost::new(tampered, None)).unwrap_err();
    assert!(error.to_string().contains("invalid"), "{error}");

    let mut wrong_owner = schema_one_state(&plan);
    wrong_owner
        .files
        .get_mut(&plan.paths.install_manifest)
        .unwrap()
        .owner_id = 0;
    let error =
        preview_uninstall(&plan.paths, false, &FakeHost::new(wrong_owner, None)).unwrap_err();
    assert!(error.to_string().contains("ownership"), "{error}");
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
    assert!(
        host.snapshot()
            .certificates
            .contains(&test_ca_fingerprint())
    );

    apply_uninstall(&plan.paths, true, &host).unwrap();

    assert!(!host.snapshot().trusted);
    assert!(host.snapshot().certificates.is_empty());
    assert!(
        !host
            .snapshot()
            .files
            .contains_key(&plan.paths.ca_certificate)
    );
}

#[test]
fn uninstall_skips_trust_removal_when_the_ca_is_already_absent() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(initial_state(), None);
    apply_setup(&plan, &host).unwrap();
    {
        let mut state = host.state.lock().unwrap();
        state.files.remove(&plan.paths.ca_certificate);
        state.certificates.clear();
        state.trusted = false;
    }

    apply_uninstall(&plan.paths, true, &host).unwrap();

    assert!(!host.snapshot().trusted);
    assert!(host.snapshot().certificates.is_empty());
}
