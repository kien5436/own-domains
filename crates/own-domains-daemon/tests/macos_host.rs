use own_domains_core::{DiagnosticStatus, Settings};
use own_domains_daemon::platform::macos::{
    CommandRunner, HostCommand, HostCommandOutput, HostFileKind, HostFileMetadata, HostFileSystem,
    HostIntegrationInspector, MacOsArchitecture, MacOsSetupOptions, NetworkInterface,
    SidecarManifest, SidecarSpec, SystemCommandRunner, SystemHostFileSystem, build_host_setup_plan,
    discover_network_interfaces, validate_host_setup_plan,
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
        covered_interfaces: vec![NetworkInterface {
            name: "en0".into(),
            primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 20)),
        }],
        architecture,
        sidecars: sidecars(),
    }
}

#[test]
fn renders_exact_resolver_dynamic_anchor_and_schema_two_manifest() {
    let mut options = options(MacOsArchitecture::Arm64);
    options.covered_interfaces.push(NetworkInterface {
        name: "en1".into(),
        primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
    });
    let plan = build_host_setup_plan(&options).unwrap();
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
            "rdr pass on lo0 inet proto udp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en0:0) port 80 -> (en0:0) port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to (en0:0) port 443 -> (en0:0) port 8443\n",
            "rdr pass on en0 inet proto udp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on en0 inet proto tcp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on en0 inet proto tcp from any to (en0:0) port 80 -> (en0:0) port 8080\n",
            "rdr pass on en0 inet proto tcp from any to (en0:0) port 443 -> (en0:0) port 8443\n",
            "rdr pass on lo0 inet proto udp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en1:0) port 80 -> (en1:0) port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to (en1:0) port 443 -> (en1:0) port 8443\n",
            "rdr pass on en1 inet proto udp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on en1 inet proto tcp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on en1 inet proto tcp from any to (en1:0) port 80 -> (en1:0) port 8080\n",
            "rdr pass on en1 inet proto tcp from any to (en1:0) port 443 -> (en1:0) port 8443\n",
        )
    );
    assert!(!anchor.content.contains("192.168.1.20"));
    assert!(!anchor.content.contains("192.168.1.21"));
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
    assert_eq!(plan.manifest.schema_version, 2);
    assert_eq!(plan.manifest.user_name, "kien");
    assert_eq!(
        plan.manifest.interfaces,
        vec!["en0".to_string(), "en1".to_string()]
    );
    let manifest = serde_json::to_value(&plan.manifest).unwrap();
    assert_eq!(manifest["schemaVersion"], 2);
    assert_eq!(manifest["interfaces"], serde_json::json!(["en0", "en1"]));
    assert!(manifest.get("lanAddress").is_none());
    assert!(manifest.get("interface").is_none());
    assert!(
        plan.manifest
            .artifacts
            .iter()
            .all(|artifact| artifact.sha256.len() == 64)
    );
    assert_eq!(plan.commands.iter().map(|command| (command.program.to_string_lossy().into_owned(), command.args.clone())).collect::<Vec<_>>(), vec![
        ("/usr/bin/security".into(), vec!["add-trusted-cert", "-d", "-r", "trustRoot", "-k", "/Library/Keychains/System.keychain", "/Users/kien/Library/Application Support/OwnDomains/caddy-data/pki/authorities/local/root.crt"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-nf", "/etc/pf.conf"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-f", "/etc/pf.conf"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-a", "dev.owndomains", "-f", "/etc/pf.anchors/dev.owndomains"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-E"].into_iter().map(String::from).collect()),
    ]);
}

#[test]
fn covers_present_interfaces_without_a_primary_ipv4_address() {
    let mut options = options(MacOsArchitecture::Arm64);
    options.covered_interfaces.push(NetworkInterface {
        name: "bridge3".into(),
        primary_ipv4: None,
    });
    let plan = build_host_setup_plan(&options).unwrap();
    let anchor = plan.artifact("/etc/pf.anchors/dev.owndomains").unwrap();

    assert!(anchor.content.contains("(bridge3:0)"));
    assert_eq!(
        plan.manifest.interfaces,
        vec!["bridge3".to_string(), "en0".to_string()]
    );
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

    let mut empty = options(MacOsArchitecture::Arm64);
    empty.covered_interfaces.clear();
    assert!(
        build_host_setup_plan(&empty)
            .unwrap_err()
            .to_string()
            .contains("eligible")
    );

    let mut uncovered = options(MacOsArchitecture::Arm64);
    uncovered.covered_interfaces[0].name = "en1".into();
    assert!(
        build_host_setup_plan(&uncovered)
            .unwrap_err()
            .to_string()
            .contains("not covered")
    );

    let mut duplicate_interfaces = options(MacOsArchitecture::Arm64);
    duplicate_interfaces
        .covered_interfaces
        .push(duplicate_interfaces.covered_interfaces[0].clone());
    assert!(
        build_host_setup_plan(&duplicate_interfaces)
            .unwrap_err()
            .to_string()
            .contains("duplicate network interface")
    );

    let mut ineligible = options(MacOsArchitecture::Arm64);
    ineligible.covered_interfaces.push(NetworkInterface {
        name: "utun0".into(),
        primary_ipv4: None,
    });
    assert!(
        build_host_setup_plan(&ineligible)
            .unwrap_err()
            .to_string()
            .contains("interface")
    );

    let mut oversized = options(MacOsArchitecture::Arm64);
    oversized.covered_interfaces[0].name = "en00000000000000".into();
    oversized.interface = "en00000000000000".into();
    assert!(
        build_host_setup_plan(&oversized)
            .unwrap_err()
            .to_string()
            .contains("interface")
    );

    // Covered names are validated independently of the selected interface.
    let mut oversized_covered = options(MacOsArchitecture::Arm64);
    oversized_covered.interface = "en0".into();
    oversized_covered.covered_interfaces[0].name = "en00000000000000".into();
    assert!(
        build_host_setup_plan(&oversized_covered)
            .unwrap_err()
            .to_string()
            .contains("interface")
    );

    let mut alias = options(MacOsArchitecture::Arm64);
    alias.covered_interfaces[0].primary_ipv4 = Some(Ipv4Addr::new(192, 168, 1, 21));
    assert!(
        build_host_setup_plan(&alias)
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

#[test]
fn discovers_only_present_eligible_interfaces_in_stable_order() {
    struct FakeIfconfig;

    impl CommandRunner for FakeIfconfig {
        fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
            assert_eq!(command.program, PathBuf::from("/sbin/ifconfig"));
            assert_eq!(command.timeout, Duration::from_secs(2));
            let output = if command.args.len() == 1 && command.args[0] == "-l" {
                "lo0 en1 bridge3 utun0 en0 awdl0 en8 en9".to_string()
            } else {
                match command.args.first().map(String::as_str) {
                    Some("en0") => {
                        "en0: flags=8863\n\tinet 192.168.1.20 netmask 0xffffff00\n".to_string()
                    }
                    Some("en1") => {
                        "en1: flags=8863\n\tinet 192.168.1.21 netmask 0xffffff00\n".to_string()
                    }
                    Some("bridge3") => "bridge3: flags=8863\n".to_string(),
                    Some("en8") => concat!(
                        "en8: flags=8863\n",
                        "\tinet 192.0.2.210 netmask 0xffffff00\n",
                        "\tinet 192.0.2.211 netmask 0xffffff00\n",
                    )
                    .to_string(),
                    // macOS lists aliases as additional `inet` lines without an
                    // `alias` token. Only the first address is the primary. An
                    // interface with no IPv4 at all is addressless.
                    Some("en9") => {
                        "en9: flags=8863\n\tinet6 fe80::1%en9 prefixlen 64\n".to_string()
                    }
                    other => panic!("unexpected interface {other:?}"),
                }
            };
            Ok(HostCommandOutput {
                status: 0,
                stdout: output.into_bytes(),
                stderr: vec![],
            })
        }
    }

    let interfaces = discover_network_interfaces(&FakeIfconfig).unwrap();
    assert_eq!(
        interfaces,
        vec![
            NetworkInterface {
                name: "bridge3".into(),
                primary_ipv4: None,
            },
            NetworkInterface {
                name: "en0".into(),
                primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 20)),
            },
            NetworkInterface {
                name: "en1".into(),
                primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
            },
            NetworkInterface {
                name: "en8".into(),
                primary_ipv4: Some(Ipv4Addr::new(192, 0, 2, 210)),
            },
            NetworkInterface {
                name: "en9".into(),
                primary_ipv4: None,
            },
        ]
    );
    let en8 = interfaces
        .iter()
        .find(|interface| interface.name == "en8")
        .unwrap();
    assert_eq!(en8.primary_ipv4, Some(Ipv4Addr::new(192, 0, 2, 210)));
    let en9 = interfaces
        .iter()
        .find(|interface| interface.name == "en9")
        .unwrap();
    assert_eq!(en9.primary_ipv4, None);
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
    pf_permission_denied: bool,
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
        if self.pf_permission_denied && program == "/sbin/pfctl" {
            return Ok(HostCommandOutput {
                status: 1,
                stdout: vec![],
                stderr: b"pfctl: /dev/pf: Permission denied".to_vec(),
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
        pf_permission_denied: false,
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
fn inspector_treats_unprivileged_pf_inspection_as_a_warning() {
    let (plan, files) = installed_host();
    let commands = FakeCommands {
        healthy: true,
        oversized: false,
        pf_permission_denied: true,
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
            .all(|diagnostic| diagnostic.status != DiagnosticStatus::Error),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "pfAnchorUnverifiable"
                && diagnostic.status == DiagnosticStatus::Warning
        }),
        "{diagnostics:?}"
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
        pf_permission_denied: false,
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
        pf_permission_denied: false,
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
