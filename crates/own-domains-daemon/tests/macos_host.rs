use own_domains_core::Settings;
use own_domains_daemon::platform::macos::{
    CommandRunner, HostCommand, HostCommandOutput, HostFileKind, HostFileMetadata, HostFileSystem,
    HostIntegrationInspector, MacOsArchitecture, MacOsSetupOptions, SidecarManifest, SidecarSpec,
    SystemCommandRunner, SystemHostFileSystem, build_host_setup_plan, validate_host_setup_plan,
};
use std::{
    collections::HashMap,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

fn sidecars() -> SidecarManifest {
    SidecarManifest {
        schema_version: 1,
        coredns: SidecarSpec {
            version: "1.14.7".into(),
            sha256: "a".repeat(64),
        },
        caddy: SidecarSpec {
            version: "2.11.7".into(),
            sha256: "b".repeat(64),
        },
    }
}

fn options(architecture: MacOsArchitecture) -> MacOsSetupOptions {
    MacOsSetupOptions {
        user_name: "kien".into(),
        user_id: 501,
        home_directory: PathBuf::from("/Users/kien"),
        state_directory: PathBuf::from("/Users/kien/Library/Application Support/OwnDomains"),
        resource_directory: PathBuf::from("/Applications/Own Domains.app/Contents/Resources"),
        backup_directory: PathBuf::from(
            "/Users/kien/Library/Application Support/OwnDomains/backups/setup-1",
        ),
        lan_address: Ipv4Addr::new(192, 168, 1, 20),
        interface: "en0".into(),
        interface_addresses: vec![Ipv4Addr::new(192, 168, 1, 20)],
        architecture,
        sidecars: sidecars(),
    }
}

#[test]
fn renders_exact_resolver_pf_manifest_and_fixed_commands() {
    let plan = build_host_setup_plan(&options(MacOsArchitecture::Arm64)).unwrap();
    let resolver = plan.artifact("/etc/resolver/own").unwrap();
    let anchor = plan.artifact("/etc/pf.anchors/dev.owndomains").unwrap();
    let pf_block = plan.artifact("/etc/pf.conf").unwrap();

    assert_eq!(resolver.content, "nameserver 127.0.0.1\nport 5354\n");
    assert_eq!(resolver.mode, 0o644);
    assert_eq!(resolver.owner_id, 0);
    assert_eq!(
        anchor.content,
        concat!(
            "rdr pass on lo0 inet proto udp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 8443\n",
            "rdr pass on lo0 inet proto udp from any to 192.168.1.20 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to 192.168.1.20 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to 192.168.1.20 port 80 -> 127.0.0.1 port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to 192.168.1.20 port 443 -> 127.0.0.1 port 8443\n",
            "rdr pass on en0 inet proto udp from any to 192.168.1.20 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on en0 inet proto tcp from any to 192.168.1.20 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on en0 inet proto tcp from any to 192.168.1.20 port 80 -> 127.0.0.1 port 8080\n",
            "rdr pass on en0 inet proto tcp from any to 192.168.1.20 port 443 -> 127.0.0.1 port 8443\n",
        )
    );
    assert_eq!(
        pf_block.content,
        concat!(
            "# BEGIN OWN DOMAINS\n",
            "rdr-anchor \"dev.owndomains\"\n",
            "load anchor \"dev.owndomains\" from \"/etc/pf.anchors/dev.owndomains\"\n",
            "# END OWN DOMAINS\n",
        )
    );
    assert!(pf_block.managed_block);
    assert_eq!(plan.manifest.schema_version, 1);
    assert_eq!(plan.manifest.user_name, "kien");
    assert_eq!(plan.manifest.interface, "en0");
    assert_eq!(plan.manifest.lan_address, Ipv4Addr::new(192, 168, 1, 20));
    assert!(
        plan.manifest
            .artifacts
            .iter()
            .all(|artifact| artifact.sha256.len() == 64)
    );
    assert_eq!(plan.commands.iter().map(|command| (command.program.to_string_lossy().into_owned(), command.args.clone())).collect::<Vec<_>>(), vec![
        ("/usr/bin/security".into(), vec!["add-trusted-cert", "-d", "-r", "trustRoot", "-k", "/Library/Keychains/System.keychain", "/Users/kien/Library/Application Support/OwnDomains/caddy-data/pki/authorities/local/root.crt"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-nf", "/etc/pf.conf"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-a", "dev.owndomains", "-f", "/etc/pf.anchors/dev.owndomains"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-E"].into_iter().map(String::from).collect()),
    ]);
}

#[test]
fn resolves_architecture_specific_absolute_runtime_paths() {
    let arm = build_host_setup_plan(&options(MacOsArchitecture::Arm64)).unwrap();
    let intel = build_host_setup_plan(&options(MacOsArchitecture::X86_64)).unwrap();

    assert!(
        arm.paths
            .coredns_binary
            .ends_with("coredns-aarch64-apple-darwin")
    );
    assert!(
        arm.paths
            .caddy_binary
            .ends_with("caddy-aarch64-apple-darwin")
    );
    assert!(
        intel
            .paths
            .coredns_binary
            .ends_with("coredns-x86_64-apple-darwin")
    );
    assert!(
        intel
            .paths
            .caddy_binary
            .ends_with("caddy-x86_64-apple-darwin")
    );
    assert!(arm.paths.all().iter().all(|path| path.is_absolute()));
}

#[test]
fn rejects_untrusted_setup_inputs_and_duplicate_artifacts() {
    let mut invalid_interface = options(MacOsArchitecture::Arm64);
    invalid_interface.interface = "en0; pfctl -d".into();
    assert!(
        build_host_setup_plan(&invalid_interface)
            .unwrap_err()
            .to_string()
            .contains("interface")
    );

    let mut mismatched = options(MacOsArchitecture::Arm64);
    mismatched.interface_addresses = vec![Ipv4Addr::new(192, 168, 1, 21)];
    assert!(
        build_host_setup_plan(&mismatched)
            .unwrap_err()
            .to_string()
            .contains("address")
    );

    let mut relative = options(MacOsArchitecture::Arm64);
    relative.state_directory = PathBuf::from("relative/state");
    assert!(
        build_host_setup_plan(&relative)
            .unwrap_err()
            .to_string()
            .contains("absolute")
    );

    let mut bad_digest = options(MacOsArchitecture::Arm64);
    bad_digest.sidecars.caddy.sha256 = "not-a-digest".into();
    assert!(
        build_host_setup_plan(&bad_digest)
            .unwrap_err()
            .to_string()
            .contains("digest")
    );

    let mut duplicate = build_host_setup_plan(&options(MacOsArchitecture::Arm64)).unwrap();
    duplicate.artifacts.push(duplicate.artifacts[0].clone());
    assert!(
        validate_host_setup_plan(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[derive(Default)]
struct FakeFiles {
    metadata: HashMap<PathBuf, HostFileMetadata>,
    content: HashMap<PathBuf, Vec<u8>>,
    hashes: HashMap<PathBuf, String>,
}

impl HostFileSystem for FakeFiles {
    fn metadata(&self, path: &Path) -> Result<Option<HostFileMetadata>, String> {
        Ok(self.metadata.get(path).cloned())
    }
    fn read(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
        let value = self
            .content
            .get(path)
            .cloned()
            .ok_or_else(|| "missing".to_string())?;
        if value.len() > max_bytes {
            Err("too large".into())
        } else {
            Ok(value)
        }
    }
    fn sha256(&self, path: &Path, _max_bytes: u64) -> Result<String, String> {
        self.hashes
            .get(path)
            .cloned()
            .ok_or_else(|| "missing".into())
    }
}

struct FakeCommands {
    healthy: bool,
    oversized: bool,
    calls: Mutex<Vec<HostCommand>>,
}

impl CommandRunner for FakeCommands {
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        self.calls.lock().unwrap().push(command.clone());
        let program = command.program.to_string_lossy();
        if self.oversized && program == "/usr/sbin/lsof" {
            return Ok(HostCommandOutput {
                status: 0,
                stdout: vec![b'x'; 65_537],
                stderr: vec![],
            });
        }
        let (status, stdout) = if program.ends_with("coredns-aarch64-apple-darwin") {
            (
                0,
                if self.healthy {
                    "CoreDNS-1.14.7"
                } else {
                    "CoreDNS-1.13.0"
                },
            )
        } else if program.ends_with("caddy-aarch64-apple-darwin") {
            (0, "v2.11.7")
        } else if program == "/sbin/pfctl" {
            (0, if self.healthy { "rdr pass on en0" } else { "" })
        } else if program == "/usr/bin/security" {
            (if self.healthy { 0 } else { 1 }, "")
        } else if program == "/usr/sbin/lsof" {
            (
                if self.healthy { 1 } else { 0 },
                if self.healthy {
                    ""
                } else {
                    "nginx 42 root TCP *:80 (LISTEN)"
                },
            )
        } else if program == "/usr/sbin/scutil" || program == "/sbin/ifconfig" {
            (
                0,
                if self.healthy {
                    "inet 192.168.1.20"
                } else {
                    "inet 192.168.1.21"
                },
            )
        } else {
            return Err(format!("unexpected executable: {program}"));
        };
        Ok(HostCommandOutput {
            status,
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        })
    }
}

fn installed_host() -> (
    own_domains_daemon::platform::macos::HostSetupPlan,
    FakeFiles,
) {
    let plan = build_host_setup_plan(&options(MacOsArchitecture::Arm64)).unwrap();
    let mut files = FakeFiles::default();

    files.metadata.insert(
        plan.paths.state_directory.clone(),
        HostFileMetadata {
            kind: HostFileKind::Other,
            owner_id: 501,
            mode: 0o700,
        },
    );
    for artifact in &plan.artifacts {
        files.metadata.insert(
            artifact.path.clone(),
            HostFileMetadata {
                kind: HostFileKind::Regular,
                owner_id: artifact.owner_id,
                mode: artifact.mode,
            },
        );
        files
            .content
            .insert(artifact.path.clone(), artifact.content.as_bytes().to_vec());
    }
    for (path, digest) in [
        (
            &plan.paths.coredns_binary,
            &plan.manifest.sidecars.coredns.sha256,
        ),
        (
            &plan.paths.caddy_binary,
            &plan.manifest.sidecars.caddy.sha256,
        ),
    ] {
        files.metadata.insert(
            path.clone(),
            HostFileMetadata {
                kind: HostFileKind::Regular,
                owner_id: 0,
                mode: 0o755,
            },
        );
        files.hashes.insert(path.clone(), digest.clone());
    }
    (plan, files)
}

#[test]
fn inspector_uses_only_fixed_bounded_commands_for_a_valid_installation() {
    let (plan, files) = installed_host();
    let commands = FakeCommands {
        healthy: true,
        oversized: false,
        calls: Mutex::new(vec![]),
    };
    let settings = Settings {
        lan_address: "192.168.1.20".into(),
        ..Default::default()
    };
    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let calls = commands.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .all(|call| call.program.is_absolute() && call.timeout <= Duration::from_secs(2))
    );
    assert_eq!(
        calls
            .iter()
            .map(|call| call.program.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec![
            "/usr/sbin/scutil",
            "/sbin/ifconfig",
            "/sbin/pfctl",
            "/usr/bin/security",
            "/Applications/Own Domains.app/Contents/Resources/coredns-aarch64-apple-darwin",
            "/Applications/Own Domains.app/Contents/Resources/caddy-aarch64-apple-darwin",
            "/usr/sbin/lsof",
        ]
    );
}

#[test]
fn inspector_reports_stable_codes_for_host_drift_and_conflicts() {
    let (plan, mut files) = installed_host();
    files
        .metadata
        .get_mut(Path::new("/etc/resolver/own"))
        .unwrap()
        .kind = HostFileKind::Symlink;
    files
        .metadata
        .get_mut(Path::new("/etc/pf.anchors/dev.owndomains"))
        .unwrap()
        .owner_id = 501;
    files
        .hashes
        .insert(plan.paths.coredns_binary.clone(), "c".repeat(64));
    let commands = FakeCommands {
        healthy: false,
        oversized: false,
        calls: Mutex::new(vec![]),
    };
    let settings = Settings {
        lan_address: "192.168.1.99".into(),
        ..Default::default()
    };
    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);
    let codes = diagnostics
        .iter()
        .map(|item| item.code.as_str())
        .collect::<Vec<_>>();
    for code in [
        "resolverMismatch",
        "pfAnchorInvalid",
        "lanIntegrationDrift",
        "pfAnchorInactive",
        "caNotTrusted",
        "sidecarDigestMismatch",
        "sidecarVersionMismatch",
        "portConflict",
    ] {
        assert!(codes.contains(&code), "missing {code}: {diagnostics:?}");
    }
}

#[test]
fn inspector_rejects_oversized_command_output() {
    let (plan, files) = installed_host();
    let commands = FakeCommands {
        healthy: true,
        oversized: true,
        calls: Mutex::new(vec![]),
    };
    let settings = Settings {
        lan_address: "192.168.1.20".into(),
        ..Default::default()
    };
    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "inspectionFailed" && item.message.contains("64 KiB"))
    );
}

fn temporary_directory() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "own-domains-system-host-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn system_host_filesystem_reports_metadata_reads_and_digests() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary_directory();
    let path = directory.join("owned.txt");
    std::fs::write(&path, b"hello").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let files = SystemHostFileSystem;

    let metadata = files.metadata(&path).unwrap().unwrap();
    assert_eq!(metadata.kind, HostFileKind::Regular);
    assert_eq!(metadata.mode, 0o600);
    assert_eq!(files.read(&path, 5).unwrap(), b"hello");
    assert!(files.read(&path, 4).unwrap_err().contains("read limit"));
    assert_eq!(
        files.sha256(&path, 5).unwrap(),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert!(files.sha256(&path, 4).unwrap_err().contains("hash limit"));
    assert!(
        files
            .metadata(&directory.join("missing"))
            .unwrap()
            .is_none()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn system_command_runner_captures_bounded_output_and_enforces_deadlines() {
    let commands = SystemCommandRunner;
    let output = commands
        .run(&HostCommand {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "printf hello; printf err >&2; exit 3".into()],
            timeout: Duration::from_secs(2),
        })
        .unwrap();
    assert_eq!(
        output,
        HostCommandOutput {
            status: 3,
            stdout: b"hello".to_vec(),
            stderr: b"err".to_vec(),
        }
    );

    let error = commands
        .run(&HostCommand {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "sleep 2".into()],
            timeout: Duration::from_millis(50),
        })
        .unwrap_err();
    assert!(error.contains("did not finish"), "{error}");
}
