use own_domains_cli::{Cli, CliCommand, execute, validate_privileged_setup_identity};
use own_domains_client::Client;
use own_domains_core::{
    ApplyResult, ApplyStatus, Diagnostic, DomainService, Settings, SettingsStore,
};
use own_domains_daemon::{DaemonState, IpcServer, RuntimeBackend, RuntimeError};
use own_domains_protocol::Command;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

struct FailingReloadRuntime(AtomicUsize);

impl RuntimeBackend for FailingReloadRuntime {
    fn apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(ApplyResult {
                desired_revision: settings.revision,
                applied_revision: settings.revision,
                status: ApplyStatus::Applied,
                message: None,
            })
        } else {
            Err(RuntimeError::Failure(
                "reload failed; check generated configuration".into(),
            ))
        }
    }

    fn stop(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn inspect(&self) -> Result<own_domains_protocol::RuntimeStatus, RuntimeError> {
        Ok(own_domains_protocol::RuntimeStatus::Running)
    }
    fn diagnose(&self, _settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError> {
        Ok(vec![])
    }
}

#[test]
fn parses_complete_command_surface() {
    for command in ["start", "stop", "status", "list", "doctor"] {
        assert!(
            Cli::try_parse_from(["own-domains", command]).is_ok(),
            "{command}"
        );
    }
    let add = Cli::try_parse_from([
        "own-domains",
        "add",
        "shop.own",
        "--port",
        "3000",
        "--revision",
        "0",
        "--https",
    ])
    .unwrap();
    assert!(matches!(add.command, CliCommand::Add { https: true, .. }));
    for command in ["edit", "enable", "disable", "remove"] {
        let mut args = vec!["own-domains", command, "shop.own", "--revision", "0"];
        if command == "edit" {
            args.extend(["--port", "4000"]);
        }
        assert!(Cli::try_parse_from(args).is_ok(), "{command}");
    }
    let setup = Cli::try_parse_from([
        "own-domains",
        "setup",
        "--lan-address",
        "192.168.1.20",
        "--interface",
        "en0",
    ])
    .unwrap();
    assert!(matches!(
        setup.command,
        CliCommand::Setup { apply: false, .. }
    ));
    let uninstall =
        Cli::try_parse_from(["own-domains", "uninstall", "--apply", "--purge"]).unwrap();
    assert!(matches!(
        uninstall.command,
        CliCommand::Uninstall {
            apply: true,
            purge: true
        }
    ));
}

#[test]
fn parser_rejects_missing_values_and_unknown_flags() {
    assert!(Cli::try_parse_from(["own-domains", "add", "shop.own", "--port", "3000"]).is_err());
    assert!(Cli::try_parse_from(["own-domains", "list", "--unknown"]).is_err());
    assert!(
        Cli::try_parse_from(["own-domains", "setup", "--lan-address", "192.168.1.20"]).is_err()
    );
    assert!(Cli::try_parse_from(["own-domains", "setup", "--interface", "en0"]).is_err());
    assert!(Cli::try_parse_from(["own-domains", "uninstall", "--unknown"]).is_err());
}

#[test]
fn privileged_setup_identity_requires_root_and_a_valid_sudo_target() {
    assert!(validate_privileged_setup_identity(true, 501, Some("kien"), Some("501")).is_err());
    assert!(validate_privileged_setup_identity(true, 0, None, Some("501")).is_err());
    assert!(validate_privileged_setup_identity(true, 0, Some("kien"), Some("root")).is_err());
    assert_eq!(
        validate_privileged_setup_identity(false, 501, None, None).unwrap(),
        None
    );
    assert_eq!(
        validate_privileged_setup_identity(true, 0, Some("kien"), Some("501"))
            .unwrap()
            .unwrap()
            .user_id,
        501
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn setup_and_uninstall_report_unsupported_platform_before_host_changes() {
    let setup = execute(
        Cli::try_parse_from([
            "own-domains",
            "setup",
            "--lan-address",
            "192.168.1.20",
            "--interface",
            "en0",
        ])
        .unwrap(),
    );
    let uninstall = execute(Cli::try_parse_from(["own-domains", "uninstall"]).unwrap());

    assert_eq!(setup.exit_code, 3);
    assert_eq!(uninstall.exit_code, 3);
    assert!(setup.stderr.contains("unsupported"));
    assert!(uninstall.stderr.contains("unsupported"));
}

#[test]
fn offline_status_and_mutation_are_truthful() {
    let state_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join(format!("od-cli-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    let status = execute(
        Cli::try_parse_from([
            "own-domains",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "status",
        ])
        .unwrap(),
    );
    assert_eq!(status.exit_code, 0);
    assert!(status.stdout.contains("stopped"));
    let add = execute(
        Cli::try_parse_from([
            "own-domains",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "add",
            "shop.own",
            "--port",
            "3000",
            "--revision",
            "0",
        ])
        .unwrap(),
    );
    assert_eq!(add.exit_code, 0);
    assert!(add.stdout.contains("saved; runtime stopped"));
}

#[test]
fn live_saved_but_unapplied_mutation_has_distinct_exit_and_output() {
    let state_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join(format!(
            "od-cli-live-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        ));
    let socket = state_dir.join("daemon.sock");
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&state_dir)),
        Arc::new(FailingReloadRuntime(AtomicUsize::new(0))),
    ));
    let server = IpcServer::bind(&socket, state).unwrap();
    let runner = thread::spawn(move || server.run().unwrap());
    let client = Client::connect(&socket).unwrap();
    client.request(Command::Start).unwrap();

    let add = execute(
        Cli::try_parse_from([
            "own-domains",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
            "add",
            "shop.own",
            "--port",
            "3000",
            "--revision",
            "0",
        ])
        .unwrap(),
    );

    assert_eq!(add.exit_code, 5);
    assert!(add.stdout.contains("revision 1"));
    assert!(
        add.stderr
            .contains("reload failed; check generated configuration")
    );

    let enable = execute(
        Cli::try_parse_from([
            "own-domains",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
            "--json",
            "enable",
            "shop.own",
            "--revision",
            "1",
        ])
        .unwrap(),
    );
    assert_eq!(enable.exit_code, 5);
    let json: serde_json::Value = serde_json::from_str(&enable.stdout).unwrap();
    assert_eq!(json["type"], "mutation");
    assert_eq!(json["payload"]["settings"]["revision"], 2);
    assert_eq!(json["payload"]["apply"]["status"], "error");
    assert_eq!(json["payload"]["apply"]["appliedRevision"], 0);
    client.request(Command::Stop).unwrap();
    runner.join().unwrap();
}
