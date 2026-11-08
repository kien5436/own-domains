//! Linux host installer, tested on every platform.
//!
//! All system effects go through the fake runner; no real mutation here.

use own_domains_daemon::platform::{
    HostInstaller, SetupRequest, UninstallRequest,
    linux::setup::LinuxHostInstaller,
    windows::setup::{CommandRunner, RunningProcess},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Responder = Box<dyn Fn(&str, &[String]) -> Result<String, String> + Send>;
type SpawnHook = Box<dyn Fn(&str, &[String]) + Send>;

struct FakeRunner {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    respond: Mutex<Responder>,
    on_spawn: Mutex<SpawnHook>,
}

impl FakeRunner {
    fn new() -> Self {
        Self::with_handler(|_, _| Ok(String::new()))
    }

    fn with_handler(
        respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            respond: Mutex::new(Box::new(respond)),
            on_spawn: Mutex::new(Box::new(|_, _| {})),
        }
    }

    fn set_spawn_hook(&self, hook: impl Fn(&str, &[String]) + Send + 'static) {
        *self.on_spawn.lock().unwrap() = Box::new(hook);
    }

    fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    fn programs(&self) -> Vec<String> {
        self.calls()
            .iter()
            .map(|(program, _)| program.clone())
            .collect()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        self.calls
            .lock()
            .unwrap()
            .push((program.to_string(), argv.clone()));
        self.respond.lock().unwrap()(program, &argv)
    }

    fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        self.calls
            .lock()
            .unwrap()
            .push((format!("spawn:{program}"), argv.clone()));
        (self.on_spawn.lock().unwrap())(program, &argv);
        Ok(Box::new(FakeChild))
    }
}

struct FakeChild;

impl RunningProcess for FakeChild {
    fn is_running(&mut self) -> Result<bool, String> {
        Ok(true)
    }
    fn terminate(&mut self) -> Result<(), String> {
        Ok(())
    }
}

fn harness() -> (Arc<FakeRunner>, LinuxHostInstaller) {
    let runner = Arc::new(FakeRunner::new());
    struct Handle(Arc<FakeRunner>);
    impl CommandRunner for Handle {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            FakeRunner::run(&self.0, program, args)
        }
        fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
            FakeRunner::spawn(&self.0, program, args)
        }
    }
    let installer = LinuxHostInstaller::with_parts(Box::new(Handle(runner.clone())), None);
    (runner, installer)
}

fn tmp_state(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("od-linux-setup-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn setup_request(state: &Path) -> SetupRequest {
    SetupRequest {
        state_directory: state.to_path_buf(),
        lan_address: "192.168.1.20".into(), // enp3s0 primary in fixtures (.21 is secondary)
        interface: "enp3s0".into(),
    }
}

#[test]
fn preview_lists_artifacts_without_running_anything() {
    let (runner, installer) = harness();
    let state = tmp_state("preview");

    let preview = installer.preview_setup(&setup_request(&state)).unwrap();

    let kinds: Vec<&str> = preview
        .actions
        .iter()
        .map(|action| action.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "linux-nftables",
            "linux-nftables-unit",
            "linux-resolver-unit",
            "linux-trust",
            "linux-manifest"
        ]
    );
    let paths: Vec<PathBuf> = preview
        .actions
        .iter()
        .map(|action| action.path.clone())
        .collect();
    assert!(paths.contains(&PathBuf::from("/etc/own-domains/nftables.conf")));
    assert!(paths.contains(&PathBuf::from(
        "/etc/systemd/system/own-domains-nftables.service"
    )));
    assert!(paths.contains(&PathBuf::from(
        "/etc/systemd/system/own-domains-resolver.service"
    )));
    assert!(paths.contains(&PathBuf::from(
        "/usr/local/share/ca-certificates/own-domains.crt"
    )));
    assert!(paths.contains(&state.join("install-manifest.json")));
    assert!(runner.calls().is_empty(), "preview must not run anything");
    assert!(
        !state.join("install-manifest.json").exists(),
        "preview must not create the manifest"
    );
}

#[test]
fn preview_uninstall_without_receipts_lists_no_actions() {
    let (runner, installer) = harness();
    let state = tmp_state("preview-uninstall");

    let preview = installer
        .preview_uninstall(&UninstallRequest {
            state_directory: state,
            purge: false,
        })
        .unwrap();

    assert!(preview.actions.is_empty());
    assert!(runner.calls().is_empty(), "preview must not run anything");
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux")
            .join(name),
    )
    .unwrap()
}

fn responding_harness(
    respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
) -> (Arc<FakeRunner>, LinuxHostInstaller, PathBuf, PathBuf) {
    let root = tmp_state("sysroot");
    let state = tmp_state("state");
    let runner = Arc::new(FakeRunner::with_handler(respond));
    // Fake staged Caddy: the spawn hook generates the CA files Caddy would
    // write, mirroring windows_setup_unit's equip_caddy.
    let caddy = state.join("caddy-fake");
    std::fs::write(&caddy, b"fake").unwrap();
    let ca_dir = state.join("caddy-data/pki/authorities/local");
    runner.set_spawn_hook(move |_, _| {
        std::fs::create_dir_all(&ca_dir).unwrap();
        for name in [
            "root.crt",
            "root.key",
            "intermediate.crt",
            "intermediate.key",
        ] {
            if name.ends_with(".crt") {
                std::fs::write(ca_dir.join(name), [0x30u8, 0x03, 0x01, 0x02, 0x03]).unwrap();
            } else {
                std::fs::write(ca_dir.join(name), b"fake-key").unwrap();
            }
        }
    });
    struct Handle(Arc<FakeRunner>);
    impl CommandRunner for Handle {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            FakeRunner::run(&self.0, program, args)
        }
        fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
            FakeRunner::spawn(&self.0, program, args)
        }
    }
    std::fs::write(
        caddy.parent().unwrap().join("manifest.json"),
        r#"{"schemaVersion":1,"coredns":{"version":"1.14.7","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"caddy":{"version":"2.11.7","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}"#,
    )
    .unwrap();
    let installer = LinuxHostInstaller::with_parts(Box::new(Handle(runner.clone())), Some(caddy))
        .with_system_root(root.clone())
        .with_target_uid(1000)
        .with_target_user_name("testuser")
        .with_required_binaries(Vec::new());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state, PermissionsExt::from_mode(0o700)).unwrap();
    }
    (runner, installer, root, state)
}

/// Responder serving the checked-in `ip` fixtures plus success for
/// everything else. Mirrors the guest layout the design targets:
/// root (`id -u` → 0), state owned by uid 1000 (`stat` → 1000).
fn fixture_responder() -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let link = fixture("ip-link.json");
    let addr = fixture("ip-addr.json");
    move |program: &str, args: &[String]| {
        if program == "id" {
            return Ok("0\n".into());
        }
        if program.ends_with("stat") {
            return Ok("1000\n".into());
        }
        if program.ends_with("ip") && args.contains(&"-j".to_string()) {
            if args.contains(&"link".to_string()) {
                return Ok(link.clone());
            }
            if args.contains(&"addr".to_string()) {
                return Ok(addr.clone());
            }
        }
        Ok(String::new())
    }
}

#[test]
fn apply_writes_nftables_and_units_then_second_apply_is_unchanged() {
    let (runner, installer, root, state) = responding_harness(fixture_responder());

    let result = installer.apply_setup(&setup_request(&state)).unwrap();
    assert_eq!(result.written.len(), 4);

    let nftables = root.join("etc/own-domains/nftables.conf");
    let nftables_unit = root.join("etc/systemd/system/own-domains-nftables.service");
    let resolver_unit = root.join("etc/systemd/system/own-domains-resolver.service");
    for path in [&nftables, &nftables_unit, &resolver_unit] {
        assert!(path.is_file(), "{} was not written", path.display());
    }
    let body = std::fs::read_to_string(&nftables).unwrap();
    assert!(body.contains("iifname \"enp3s0\""), "{body}");
    assert!(body.contains("redirect to :5354"), "{body}");
    assert_eq!(
        std::fs::read_to_string(&nftables_unit).unwrap(),
        own_domains_daemon::platform::linux::systemd::render_nftables_unit()
    );
    assert_eq!(
        std::fs::read_to_string(&resolver_unit).unwrap(),
        own_domains_daemon::platform::linux::systemd::render_resolver_unit()
    );

    let programs = runner.programs();
    assert!(programs.contains(&"/usr/sbin/nft".to_string()));
    assert!(programs.contains(&"/usr/bin/systemctl".to_string()));

    for name in [
        "nftables.json",
        "nftables-unit.json",
        "resolver-unit.json",
        "trust.json",
    ] {
        assert!(
            state.join("applied").join(name).is_file(),
            "{name} receipt missing"
        );
    }

    let calls_after_first = runner.calls().len();

    let second = installer.apply_setup(&setup_request(&state)).unwrap();
    assert!(second.written.is_empty());
    assert_eq!(second.unchanged.len(), 4);
    // Preflight re-runs its read-only probes, but a fully unchanged
    // apply performs zero host mutations.
    let mutations: Vec<String> = runner.calls()[calls_after_first..]
        .iter()
        .map(|(program, _)| program.clone())
        .filter(|program| {
            program.ends_with("nft")
                || program.ends_with("systemctl")
                || program.ends_with("update-ca-certificates")
                || program.ends_with("chown")
                || program.starts_with("spawn:")
        })
        .collect();
    assert!(mutations.is_empty(), "unexpected mutations: {mutations:?}");

    // LAN address persisted without resetting anything else.
    let settings = own_domains_core::SettingsStore::new(&state).load().unwrap();
    assert_eq!(settings.lan_address, "192.168.1.20");
}

#[test]
fn uninstall_reverses_units_and_table_and_deletes_receipts() {
    let (runner, installer, root, state) = responding_harness(fixture_responder());
    installer.apply_setup(&setup_request(&state)).unwrap();
    let calls_after_apply = runner.calls().len();

    let result = installer
        .apply_uninstall(&UninstallRequest {
            state_directory: state.clone(),
            purge: false,
        })
        .unwrap();

    assert!(result.written.is_empty());
    for path in [
        root.join("etc/own-domains/nftables.conf"),
        root.join("etc/systemd/system/own-domains-nftables.service"),
        root.join("etc/systemd/system/own-domains-resolver.service"),
    ] {
        assert!(!path.exists(), "{} was not removed", path.display());
    }
    for name in [
        "nftables.json",
        "nftables-unit.json",
        "resolver-unit.json",
        "trust.json",
    ] {
        assert!(
            !state.join("applied").join(name).exists(),
            "{name} receipt not deleted"
        );
    }
    let programs: Vec<String> = runner.calls()[calls_after_apply..]
        .iter()
        .map(|(program, _)| program.clone())
        .collect();
    assert!(programs.contains(&"/usr/bin/systemctl".to_string()));
    assert!(programs.contains(&"/usr/sbin/nft".to_string()));

    // Second uninstall is a no-op success.
    installer
        .apply_uninstall(&UninstallRequest {
            state_directory: state,
            purge: false,
        })
        .unwrap();
}

#[test]
fn apply_failure_leaves_no_receipts_and_removes_partial_files() {
    let (_runner, installer, root, state) = responding_harness(|program, args| {
        if program.ends_with("nft") {
            return Err("nft -c -f: syntax error".into());
        }
        fixture_responder()(program, args)
    });

    assert!(installer.apply_setup(&setup_request(&state)).is_err());
    assert!(
        !state.join("applied").join("nftables.json").exists(),
        "failed apply must not leave receipts"
    );
    assert!(
        !root.join("etc/own-domains/nftables.conf").exists(),
        "failed apply must roll back partial files"
    );
}

#[test]
fn trust_installs_cert_and_records_fingerprint() {
    let (runner, installer, root, state) = responding_harness(fixture_responder());

    installer.apply_setup(&setup_request(&state)).unwrap();

    let cert = root.join("usr/local/share/ca-certificates/own-domains.crt");
    assert!(cert.is_file(), "trust cert was not installed");
    let der = std::fs::read(&cert).unwrap();
    let expected = own_domains_daemon::platform::linux::trust::certificate_sha256(&der);
    let receipt: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(state.join("applied/trust.json")).unwrap())
            .unwrap();
    assert_eq!(
        receipt.get("fingerprint").and_then(|value| value.as_str()),
        Some(expected.as_str())
    );
    assert!(
        runner
            .programs()
            .iter()
            .any(|program| program.ends_with("update-ca-certificates")),
        "trust apply must refresh the system bundle"
    );
}

#[test]
fn uninstall_removes_trust_after_fingerprint_match() {
    let (runner, installer, root, state) = responding_harness(fixture_responder());
    installer.apply_setup(&setup_request(&state)).unwrap();
    let cert = root.join("usr/local/share/ca-certificates/own-domains.crt");
    assert!(cert.is_file());

    installer
        .apply_uninstall(&UninstallRequest {
            state_directory: state.clone(),
            purge: false,
        })
        .unwrap();

    assert!(!cert.exists(), "trust cert was not removed");
    assert!(
        !state.join("applied/trust.json").exists(),
        "trust receipt not deleted"
    );
    let updates = runner
        .calls()
        .iter()
        .filter(|(program, _)| program.ends_with("update-ca-certificates"))
        .count();
    assert_eq!(
        updates, 2,
        "expected update-ca-certificates on apply and uninstall"
    );
}

#[test]
fn uninstall_refuses_trust_on_fingerprint_mismatch() {
    let (_runner, installer, root, state) = responding_harness(fixture_responder());
    installer.apply_setup(&setup_request(&state)).unwrap();

    std::fs::write(
        root.join("usr/local/share/ca-certificates/own-domains.crt"),
        [0x30u8, 0x03, 0x05, 0x06, 0x07],
    )
    .unwrap();

    let error = installer
        .apply_uninstall(&UninstallRequest {
            state_directory: state.clone(),
            purge: false,
        })
        .unwrap_err();
    assert!(
        error.message.contains("caFingerprintMismatch"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        root.join("usr/local/share/ca-certificates/own-domains.crt")
            .is_file(),
        "foreign cert must be left alone"
    );
    assert!(
        state.join("applied/trust.json").is_file(),
        "trust receipt must survive a refused removal"
    );
}

fn setup_request_with(state: &std::path::Path, lan: &str, interface: &str) -> SetupRequest {
    SetupRequest {
        state_directory: state.to_path_buf(),
        lan_address: lan.into(),
        interface: interface.into(),
    }
}

#[test]
fn apply_rejects_ineligible_interface() {
    let (_runner, installer, _root, state) = responding_harness(fixture_responder());

    let error = installer
        .apply_setup(&setup_request_with(&state, "172.17.0.1", "docker0"))
        .unwrap_err();
    assert!(
        error.message.contains("docker0"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn apply_rejects_lan_not_on_the_interface() {
    let (_runner, installer, _root, state) = responding_harness(fixture_responder());

    let error = installer
        .apply_setup(&setup_request_with(&state, "192.168.1.21", "enp3s0"))
        .unwrap_err();
    assert!(
        error.message.contains("192.168.1.21"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn apply_writes_install_manifest_last() {
    let (_runner, installer, _root, state) = responding_harness(fixture_responder());

    installer.apply_setup(&setup_request(&state)).unwrap();

    let manifest_path = state.join("install-manifest.json");
    assert!(manifest_path.is_file(), "manifest was not written");
    let bytes = std::fs::read(&manifest_path).unwrap();
    let manifest =
        own_domains_daemon::platform::linux::manifest::parse_linux_manifest(&bytes).unwrap();
    assert_eq!(manifest.platform, "linux");
    assert!(!manifest.interfaces.is_empty());
    assert!(manifest.interfaces.contains(&"enp3s0".to_string()));
    assert_eq!(manifest.artifacts.len(), 4);
    let trust_receipt: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(state.join("applied/trust.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest.ca_fingerprint_sha256,
        trust_receipt
            .get("fingerprint")
            .and_then(|value| value.as_str())
            .unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&manifest_path).unwrap().mode() & 0o777,
            0o600,
            "manifest must be owner-only"
        );
    }
}

#[test]
fn uninstall_removes_manifest_last() {
    let (_runner, installer, _root, state) = responding_harness(fixture_responder());
    installer.apply_setup(&setup_request(&state)).unwrap();
    let manifest_path = state.join("install-manifest.json");
    assert!(manifest_path.is_file());

    installer
        .apply_uninstall(&UninstallRequest {
            state_directory: state.clone(),
            purge: false,
        })
        .unwrap();

    assert!(!manifest_path.exists(), "manifest was not removed");
    assert!(
        !state.join("applied").exists()
            || state.join("applied").read_dir().unwrap().next().is_none()
    );
}

#[test]
fn uninstall_refuses_corrupt_manifest() {
    let (_runner, installer, _root, state) = responding_harness(fixture_responder());
    installer.apply_setup(&setup_request(&state)).unwrap();
    std::fs::write(state.join("install-manifest.json"), b"{not json").unwrap();

    let error = installer
        .apply_uninstall(&UninstallRequest {
            state_directory: state.clone(),
            purge: false,
        })
        .unwrap_err();
    assert!(
        error.message.contains("install-manifest.json"),
        "unexpected error: {}",
        error.message
    );
}

fn preflight_harness(
    respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
) -> (
    std::sync::Arc<FakeRunner>,
    LinuxHostInstaller,
    std::path::PathBuf,
) {
    let (runner, installer, _root, state) = responding_harness(respond);
    (runner, installer, state)
}

#[test]
fn preflight_rejects_non_root_before_any_mutation() {
    let (_runner, installer, state) = preflight_harness(|program, args| {
        if program == "id" {
            return Ok("1000\n".into());
        }
        fixture_responder()(program, args)
    });

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("root") || error.message.contains("sudo"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn preflight_rejects_relative_state_dir() {
    let (_runner, installer, _state) = preflight_harness(fixture_responder());
    let request = setup_request(std::path::Path::new("relative/state"));

    let error = installer.apply_setup(&request).unwrap_err();
    assert!(
        error.message.contains("absolute"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn preflight_rejects_missing_binaries() {
    let (_runner, installer, state) = preflight_harness(fixture_responder());
    let installer =
        installer_with_bins(installer, vec!["/nonexistent/required-binary".to_string()]);

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("/nonexistent/required-binary"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn preflight_rejects_foreign_nftables_file() {
    let (_runner, installer, root, state) = responding_harness(fixture_responder());
    std::fs::create_dir_all(root.join("etc/own-domains")).unwrap();
    std::fs::write(root.join("etc/own-domains/nftables.conf"), "foreign").unwrap();

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("foreign"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

fn installer_with_bins(installer: LinuxHostInstaller, bins: Vec<String>) -> LinuxHostInstaller {
    installer.with_required_binaries(bins)
}

/// `ip` fixtures plus one extra eligible interface carrying 10.0.0.5.
fn drifted_ip_responder() -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let mut links: Vec<serde_json::Value> = serde_json::from_str(&fixture("ip-link.json")).unwrap();
    let mut addrs: Vec<serde_json::Value> = serde_json::from_str(&fixture("ip-addr.json")).unwrap();
    links.push(serde_json::json!({"ifindex": 99, "ifname": "enp5s0"}));
    addrs.push(serde_json::json!({
        "ifindex": 99,
        "ifname": "enp5s0",
        "addr_info": [{"family": "inet", "local": "10.0.0.5", "prefixlen": 24, "scope": "global"}],
    }));
    let link = serde_json::to_string(&links).unwrap();
    let addr = serde_json::to_string(&addrs).unwrap();
    move |program: &str, args: &[String]| {
        if program == "id" {
            return Ok("0\n".into());
        }
        if program.ends_with("stat") {
            return Ok("1000\n".into());
        }
        if program.ends_with("ip") && args.contains(&"-j".to_string()) {
            if args.contains(&"link".to_string()) {
                return Ok(link.clone());
            }
            if args.contains(&"addr".to_string()) {
                return Ok(addr.clone());
            }
        }
        Ok(String::new())
    }
}

#[test]
fn reapply_with_new_interface_backs_up_and_updates() {
    let (runner, installer, root, state) = responding_harness(fixture_responder());
    installer.apply_setup(&setup_request(&state)).unwrap();
    let before = std::fs::read(root.join("etc/own-domains/nftables.conf")).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains("enp5s0"));

    // Fresh installer over the same state but a wider interface set.
    let runner2 = std::sync::Arc::new(FakeRunner::with_handler(drifted_ip_responder()));
    struct Handle2(std::sync::Arc<FakeRunner>);
    impl CommandRunner for Handle2 {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            FakeRunner::run(&self.0, program, args)
        }
        fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
            FakeRunner::spawn(&self.0, program, args)
        }
    }
    let caddy = state.join("caddy-fake");
    runner2.set_spawn_hook({
        let state = state.clone();
        move |_, _| {
            let ca_dir = state.join("caddy-data/pki/authorities/local");
            std::fs::create_dir_all(&ca_dir).unwrap();
            for name in [
                "root.crt",
                "root.key",
                "intermediate.crt",
                "intermediate.key",
            ] {
                if name.ends_with(".crt") {
                    std::fs::write(ca_dir.join(name), [0x30u8, 0x03, 0x01, 0x02, 0x03]).unwrap();
                } else {
                    std::fs::write(ca_dir.join(name), b"fake-key").unwrap();
                }
            }
        }
    });
    let installer2 =
        LinuxHostInstaller::with_parts(Box::new(Handle2(runner2.clone())), Some(caddy))
            .with_system_root(root.clone())
            .with_target_uid(1000)
            .with_target_user_name("testuser")
            .with_required_binaries(Vec::new());

    let result = installer2.apply_setup(&setup_request(&state)).unwrap();
    assert_eq!(
        result.written.len(),
        1,
        "only nftables drifted: {:?}",
        result.written
    );
    assert_eq!(result.unchanged.len(), 3);

    let after = std::fs::read(root.join("etc/own-domains/nftables.conf")).unwrap();
    assert!(String::from_utf8_lossy(&after).contains("enp5s0"));

    // The replaced ruleset survives in the root-owned backup dir.
    let backup = root.join("etc/own-domains/backups/nftables.conf.prev");
    assert!(backup.is_file(), "backup was not written");
    assert_eq!(std::fs::read(&backup).unwrap(), before);

    // The drift reloaded the units.
    assert!(
        runner2
            .programs()
            .contains(&"/usr/bin/systemctl".to_string()),
        "drifted apply must reload units"
    );
    let _ = runner;
}

#[test]
fn rollback_reports_unresolved_items_and_keeps_nothing_half_written() {
    let root = tmp_state("rollback-sysroot");
    let state = tmp_state("rollback-state");
    let caddy = state.join("caddy-fake");
    std::fs::write(&caddy, b"fake").unwrap();
    std::fs::write(
        caddy.parent().unwrap().join("manifest.json"),
        r#"{"schemaVersion":1,"coredns":{"version":"1.14.7","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"caddy":{"version":"2.11.7","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}"#,
    )
    .unwrap();
    let runner = std::sync::Arc::new(FakeRunner::with_handler({
        let root = root.clone();
        move |program: &str, args: &[String]| {
            if program == "id" {
                return Ok("0\n".into());
            }
            if program.ends_with("stat") {
                return Ok("1000\n".into());
            }
            if program.ends_with("systemctl")
                && args.first().is_some_and(|arg| arg == "daemon-reload")
            {
                // Sabotage one rollback removal: swap the file for a
                // non-empty directory so its deletion fails.
                let target = root.join("etc/own-domains/nftables.conf");
                let _ = std::fs::remove_file(&target);
                let _ = std::fs::create_dir_all(&target);
                let _ = std::fs::write(target.join("blocker"), b"x");
                return Err("daemon-reload failed".into());
            }
            fixture_responder()(program, args)
        }
    }));
    runner.set_spawn_hook({
        let state = state.clone();
        move |_, _| {
            let ca_dir = state.join("caddy-data/pki/authorities/local");
            std::fs::create_dir_all(&ca_dir).unwrap();
            for name in [
                "root.crt",
                "root.key",
                "intermediate.crt",
                "intermediate.key",
            ] {
                if name.ends_with(".crt") {
                    std::fs::write(ca_dir.join(name), [0x30u8, 0x03, 0x01, 0x02, 0x03]).unwrap();
                } else {
                    std::fs::write(ca_dir.join(name), b"fake-key").unwrap();
                }
            }
        }
    });
    struct Handle3(std::sync::Arc<FakeRunner>);
    impl CommandRunner for Handle3 {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            FakeRunner::run(&self.0, program, args)
        }
        fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
            FakeRunner::spawn(&self.0, program, args)
        }
    }
    let installer = LinuxHostInstaller::with_parts(Box::new(Handle3(runner.clone())), Some(caddy))
        .with_system_root(root.clone())
        .with_target_uid(1000)
        .with_target_user_name("testuser")
        .with_required_binaries(Vec::new());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state, PermissionsExt::from_mode(0o700)).unwrap();
    }

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("daemon-reload"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error
            .unresolved
            .iter()
            .any(|item| item.contains("nftables.conf")),
        "rollback failure must be reported, got: {:?}",
        error.unresolved
    );
    // The unrecoverable entry keeps its receipt so a later uninstall
    // can resume; every other receipt is gone.
    assert!(
        state.join("applied/nftables.json").is_file(),
        "failed entry's receipt must survive for uninstall resume"
    );
    for name in ["nftables-unit.json", "resolver-unit.json", "trust.json"] {
        assert!(
            !state.join("applied").join(name).exists(),
            "{name} receipt must not survive rollback"
        );
    }
}

fn ufw_responder(
    status_output: &str,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let status_output = status_output.to_string();
    move |program: &str, args: &[String]| {
        if program.ends_with("ufw") {
            return Ok(status_output.clone());
        }
        fixture_responder()(program, args)
    }
}

const UFW_ACTIVE_DENY_MISSING_UDP: &str = "Status: active\n\nTo                         Action      From\n--                         ------      ----\n5354/tcp                   ALLOW       Anywhere\n8080/tcp                   ALLOW       Anywhere\n8443/tcp                   ALLOW       Anywhere\n\nDefault: deny (incoming), allow (outgoing), disabled (routed)\n";

const UFW_ACTIVE_DENY_COMPLETE: &str = "Status: active\n\nTo                         Action      From\n--                         ------      ----\n5354/tcp                   ALLOW       Anywhere\n5354/udp                   ALLOW       Anywhere\n8080/tcp                   ALLOW       Anywhere\n8443/tcp                   ALLOW       Anywhere\n\nDefault: deny (incoming), allow (outgoing), disabled (routed)\n";

#[test]
fn preflight_rejects_active_ufw_with_missing_high_port() {
    let (_runner, installer, _root, state) =
        responding_harness(ufw_responder(UFW_ACTIVE_DENY_MISSING_UDP));

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("ufw"),
        "unexpected error: {}",
        error.message
    );
    // Only the gap is reported; already-allowed ports are not repeated.
    assert!(
        error.message.contains("sudo ufw allow 5354/udp"),
        "missing remedy: {}",
        error.message
    );
    for allowed in [
        "sudo ufw allow 5354/tcp",
        "sudo ufw allow 8080/tcp",
        "sudo ufw allow 8443/tcp",
    ] {
        assert!(
            !error.message.contains(allowed),
            "already-allowed {allowed} must not be repeated: {}",
            error.message
        );
    }
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn preflight_accepts_allowed_ufw() {
    let (_runner, installer, _root, state) =
        responding_harness(ufw_responder(UFW_ACTIVE_DENY_COMPLETE));

    installer.apply_setup(&setup_request(&state)).unwrap();
}

#[test]
fn preflight_accepts_inactive_ufw() {
    let (_runner, installer, _root, state) =
        responding_harness(ufw_responder("Status: inactive\n"));
    installer.apply_setup(&setup_request(&state)).unwrap();
}

#[test]
fn preflight_accepts_missing_ufw() {
    let (_runner, installer, _root, state) = responding_harness(|program, args| {
        if program.ends_with("ufw") {
            return Err("ufw: command not found".into());
        }
        fixture_responder()(program, args)
    });

    installer.apply_setup(&setup_request(&state)).unwrap();
}

fn firewalld_responder(
    state: &str,
    zone: &str,
    ports: &str,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let state = state.to_string();
    let zone = zone.to_string();
    let ports = ports.to_string();
    move |program: &str, args: &[String]| {
        if program.ends_with("firewall-cmd") {
            if args.contains(&"--state".to_string()) {
                return Ok(state.clone());
            }
            if args.contains(&"--get-default-zone".to_string()) {
                return Ok(zone.clone());
            }
            if args.contains(&"--list-ports".to_string()) {
                return Ok(ports.clone());
            }
            return Ok(String::new());
        }
        fixture_responder()(program, args)
    }
}

#[test]
fn preflight_rejects_firewalld_with_missing_high_port() {
    let (_runner, installer, _root, state) = responding_harness(firewalld_responder(
        "running\n",
        "public\n",
        "8080/tcp 8443/tcp\n",
    ));

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("firewalld") || error.message.contains("firewall-cmd"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error.message.contains("--add-port=5354/tcp"),
        "missing remedy: {}",
        error.message
    );
    assert!(
        error.message.contains("--reload"),
        "missing reload step: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn preflight_accepts_allowed_firewalld() {
    let (_runner, installer, _root, state) = responding_harness(firewalld_responder(
        "running\n",
        "public\n",
        "5354/tcp 5354/udp 8080/tcp 8443/tcp\n",
    ));

    installer.apply_setup(&setup_request(&state)).unwrap();
}

#[test]
fn preflight_accepts_stopped_firewalld() {
    let (_runner, installer, _root, state) =
        responding_harness(firewalld_responder("not running\n", "public\n", "\n"));

    installer.apply_setup(&setup_request(&state)).unwrap();
}

#[test]
fn flush_ruleset_unit_reports_warning_not_failure() {
    let (_runner, installer, root, state) = responding_harness(|program, args| {
        if program.ends_with("systemctl")
            && args.contains(&"UnitFileState".to_string())
            && args.iter().any(|arg| arg == "nftables.service")
        {
            return Ok("enabled\n".into());
        }
        fixture_responder()(program, args)
    });
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(
        root.join("etc/nftables.conf"),
        "flush ruleset\ntable ip filter {}\n",
    )
    .unwrap();

    let result = installer.apply_setup(&setup_request(&state)).unwrap();
    assert_eq!(result.written.len(), 4, "{result:?}");
    assert_eq!(result.warnings.len(), 1, "{result:?}");
    assert!(
        result.warnings[0].contains("nftablesServiceConflict"),
        "{:?}",
        result.warnings
    );
}

#[test]
fn no_flush_ruleset_means_no_warning() {
    let (_runner, installer, _root, state) = responding_harness(fixture_responder());

    let result = installer.apply_setup(&setup_request(&state)).unwrap();
    assert!(result.warnings.is_empty(), "{result:?}");
}

#[test]
fn preflight_rejects_global_own_domain() {
    let (_runner, installer, _root, state) = responding_harness(|program, args| {
        if program.ends_with("resolvectl") && args.first().is_some_and(|arg| arg == "status") {
            return Ok("Global\n       Protocols: LLMNR=resolve\n        DNS Domain: ~own\n\nLink 2 (enp3s0)\n      Current DNS Server: 192.168.1.20\n".into());
        }
        fixture_responder()(program, args)
    });

    let error = installer.apply_setup(&setup_request(&state)).unwrap_err();
    assert!(
        error.message.contains("global"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error.message.contains("resolved"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        !state.join("applied").exists(),
        "rejected apply must not create receipts"
    );
}

#[test]
fn preflight_accepts_clean_global_block() {
    let (_runner, installer, _root, state) = responding_harness(|program, args| {
        if program.ends_with("resolvectl") && args.first().is_some_and(|arg| arg == "status") {
            return Ok("Global\n       Protocols: LLMNR=resolve\n       DNS Servers: 192.168.1.1\n\nLink 2 (enp3s0)\n      Current DNS Server: 192.168.1.20\n".into());
        }
        fixture_responder()(program, args)
    });

    installer.apply_setup(&setup_request(&state)).unwrap();
}
