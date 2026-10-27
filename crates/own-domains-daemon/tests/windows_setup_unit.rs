//! Windows host installer, tested on every platform.
//!
//! All system effects go through the fake runner; no real mutation here.

use own_domains_daemon::platform::{
    HostInstaller, SetupRequest, UninstallRequest,
    windows::setup::{CommandRunner, SessionProbe, WindowsHostInstaller},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

type Responder = Box<dyn Fn(&str, &[String]) -> Result<String, String> + Send>;

struct FakeRunner {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    respond: Mutex<Responder>,
}

impl FakeRunner {
    fn with_handler(
        respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            respond: Mutex::new(Box::new(respond)),
        }
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
}

struct FakeSession {
    interactive: bool,
}

impl SessionProbe for FakeSession {
    fn is_interactive(&self) -> bool {
        self.interactive
    }
}

/// Default answers every harness needs: the pre-existing-cert thumbprint
/// query and auto-naming NRPT lookups (absent, then created). Test-specific
/// behavior delegates through `respond`.
fn chain(
    respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let queries = Arc::new(AtomicUsize::new(0));
    move |program, args| {
        if program == "powershell" {
            if args.iter().any(|arg| arg.contains("X509Certificate2")) {
                return Ok("AB12CD34EF56\n".into());
            }
            if args.iter().any(|arg| arg.contains("ExpandProperty Name")) {
                return Ok(if queries.fetch_add(1, Ordering::SeqCst) == 0 {
                    String::new()
                } else {
                    "RULE-AUTO-1\n".into()
                });
            }
        }
        respond(program, args)
    }
}

struct Harness {
    state: PathBuf,
    runner: Arc<FakeRunner>,
    installer: WindowsHostInstaller,
}

fn harness(interactive: bool) -> Harness {
    let state = std::env::temp_dir().join(format!("od-win-setup-{}", uuid::Uuid::new_v4()));
    let runner = Arc::new(FakeRunner::with_handler(chain(|_, _| Ok(String::new()))));
    let installer = WindowsHostInstaller::with_parts(
        Box::new(FakeRunnerHandle(runner.clone())),
        Box::new(FakeSession { interactive }),
    );
    Harness {
        state,
        runner,
        installer,
    }
}

/// `Arc<FakeRunner>` cannot implement the runner directly (foreign trait is
/// local, but the test wants shared observation), so delegate.
struct FakeRunnerHandle(Arc<FakeRunner>);

impl CommandRunner for FakeRunnerHandle {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        self.0.run(program, args)
    }
}

fn setup_request(state: &Path) -> SetupRequest {
    std::fs::create_dir_all(state).unwrap();
    std::fs::write(state.join("ca.crt"), b"fake-cert").unwrap();
    SetupRequest {
        state_directory: state.to_path_buf(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    }
}

fn receipt_names(preview: &own_domains_daemon::platform::SetupPreview) -> Vec<String> {
    preview
        .actions
        .iter()
        .map(|action| {
            action
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn preview_lists_receipts_without_running_anything() {
    let harness = harness(true);
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    let preview = harness.installer.preview_setup(&request).unwrap();
    assert_eq!(
        receipt_names(&preview),
        vec!["nrpt.json", "trust.json", "firewall.json"]
    );
    assert!(harness.runner.calls().is_empty());
}

#[test]
fn apply_writes_receipts_and_second_apply_is_unchanged() {
    let harness = harness(true);
    let request = setup_request(&harness.state);
    let first = harness.installer.apply_setup(&request).unwrap();
    assert_eq!(first.written.len(), 3);
    for name in ["nrpt.json", "trust.json", "firewall.json"] {
        assert!(harness.state.join("applied").join(name).is_file(), "{name}");
    }
    let programs = harness.runner.programs();
    assert!(programs.contains(&"powershell".to_string()), "{programs:?}");
    assert!(programs.contains(&"certutil".to_string()), "{programs:?}");
    assert!(programs.contains(&"netsh".to_string()), "{programs:?}");

    let calls_before = harness.runner.calls().len();
    let second = harness.installer.apply_setup(&request).unwrap();
    assert!(second.written.is_empty());
    assert_eq!(second.unchanged.len(), 3);
    assert_eq!(harness.runner.calls().len(), calls_before);
}

#[test]
fn uninstall_reverses_and_deletes_receipts() {
    let harness = harness(true);
    let request = setup_request(&harness.state);
    harness.installer.apply_setup(&request).unwrap();
    let calls_after_apply = harness.runner.calls().len();

    let preview = harness
        .installer
        .preview_uninstall(&UninstallRequest {
            state_directory: harness.state.clone(),
            purge: false,
        })
        .unwrap();
    assert_eq!(receipt_names(&preview).len(), 3);

    harness
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: harness.state.clone(),
            purge: false,
        })
        .unwrap();
    assert!(harness.runner.calls().len() > calls_after_apply);
    assert!(
        std::fs::read_dir(harness.state.join("applied"))
            .unwrap()
            .next()
            .is_none()
    );

    // Empty state uninstalls to an empty preview.
    let preview = harness
        .installer
        .preview_uninstall(&UninstallRequest {
            state_directory: harness.state.clone(),
            purge: false,
        })
        .unwrap();
    assert!(preview.actions.is_empty());
}

#[test]
fn non_interactive_session_fails_without_mutation() {
    let harness = harness(false);
    let request = setup_request(&harness.state);
    let error = harness.installer.apply_setup(&request).unwrap_err();
    assert!(error.message.contains("interactive"), "{}", error.message);
    assert!(harness.runner.calls().is_empty());
    assert!(!harness.state.join("applied").exists());
}

fn nrpt_harness(
    respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
) -> Harness {
    let state = std::env::temp_dir().join(format!("od-win-setup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("ca.crt"), b"fake-cert").unwrap();
    let runner = Arc::new(FakeRunner::with_handler(move |program, args| {
        if program == "powershell" && args.iter().any(|arg| arg.contains("X509Certificate2")) {
            return Ok("AB12CD34EF56\n".into());
        }
        respond(program, args)
    }));
    let installer = WindowsHostInstaller::with_parts(
        Box::new(FakeRunnerHandle(runner.clone())),
        Box::new(FakeSession { interactive: true }),
    );
    Harness {
        state,
        runner,
        installer,
    }
}

fn powershell_commands(harness: &Harness) -> Vec<String> {
    harness
        .runner
        .calls()
        .iter()
        .filter(|(program, _)| program == "powershell")
        .map(|(_, args)| args.join(" "))
        .collect()
}

#[test]
fn nrpt_apply_checks_then_adds_with_exact_argv() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let queries = Arc::new(AtomicUsize::new(0));
    let queries_seen = queries.clone();
    let harness = nrpt_harness(move |_, args| {
        if args
            .iter()
            .any(|arg| arg.contains("Select-Object -ExpandProperty Name"))
        {
            // First lookup: absent. Second lookup (after Add): created.
            Ok(if queries_seen.fetch_add(1, Ordering::SeqCst) == 0 {
                String::new()
            } else {
                "RULE-NAME-1\n".into()
            })
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    harness.installer.apply_setup(&request).unwrap();
    let commands = powershell_commands(&harness);
    let adds: Vec<_> = commands
        .iter()
        .filter(|command| command.contains("Add-DnsClientNrptRule"))
        .collect();
    assert_eq!(adds.len(), 1, "{commands:?}");
    assert!(
        adds[0].contains("-Namespace .own")
            && adds[0].contains("-NameServers 127.0.0.1")
            && adds[0].contains("-DisplayName OwnDomains"),
        "{commands:?}"
    );
    assert_eq!(queries.load(Ordering::SeqCst), 2);
    let receipt: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(harness.state.join("applied").join("nrpt.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["namespace"], ".own");
    assert_eq!(receipt["name"], "RULE-NAME-1");
}

#[test]
fn nrpt_apply_skips_add_when_rule_exists() {
    let harness = nrpt_harness(|_, args| {
        if args.iter().any(|arg| arg.contains("Get-DnsClientNrptRule")) {
            Ok("RULE-NAME-9\n".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    harness.installer.apply_setup(&request).unwrap();
    let commands = powershell_commands(&harness);
    assert!(
        !commands
            .iter()
            .any(|command| command.contains("Add-DnsClientNrptRule")),
        "{commands:?}"
    );
    let receipt: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(harness.state.join("applied").join("nrpt.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["name"], "RULE-NAME-9");
}

#[test]
fn nrpt_add_failure_aborts_without_receipt() {
    let harness = nrpt_harness(|_, args| {
        if args.iter().any(|arg| arg.contains("Add-DnsClientNrptRule")) {
            Err("Access is denied.".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    let error = harness.installer.apply_setup(&request).unwrap_err();
    assert!(
        error.message.contains("Access is denied"),
        "{}",
        error.message
    );
    assert!(!harness.state.join("applied").join("nrpt.json").exists());
}

#[test]
fn nrpt_remove_uses_receipt_name_and_skips_when_absent() {
    let harness = nrpt_harness(|_, args| {
        if args.iter().any(|arg| arg.contains("Get-DnsClientNrptRule")) {
            Ok("{RULE-NAME-9}\nOTHER-RULE\n".into())
        } else {
            Ok(String::new())
        }
    });
    std::fs::create_dir_all(harness.state.join("applied")).unwrap();
    std::fs::write(
        harness.state.join("applied").join("nrpt.json"),
        r#"{"namespace":".own","nameservers":["127.0.0.1"],"name":"{RULE-NAME-9}"}"#,
    )
    .unwrap();
    harness
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: harness.state.clone(),
            purge: false,
        })
        .unwrap();
    let commands = powershell_commands(&harness);
    assert!(
        commands.iter().any(
            |command| command.contains("Remove-DnsClientNrptRule -Name '{RULE-NAME-9}' -Force")
        ),
        "{commands:?}"
    );

    // Absent rule: no Remove call, receipt still deleted.
    let idle = nrpt_harness(|_, _| Ok(String::new()));
    std::fs::create_dir_all(idle.state.join("applied")).unwrap();
    std::fs::write(
        idle.state.join("applied").join("nrpt.json"),
        r#"{"namespace":".own","nameservers":["127.0.0.1"],"name":"{RULE-NAME-9}"}"#,
    )
    .unwrap();
    idle.installer
        .apply_uninstall(&UninstallRequest {
            state_directory: idle.state.clone(),
            purge: false,
        })
        .unwrap();
    let commands = powershell_commands(&idle);
    assert!(
        !commands
            .iter()
            .any(|command| command.contains("Remove-DnsClientNrptRule")),
        "{commands:?}"
    );
    assert!(!idle.state.join("applied").join("nrpt.json").exists());
}

fn firewall_harness(
    respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
) -> Harness {
    let state = std::env::temp_dir().join(format!("od-win-setup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("ca.crt"), b"fake-cert").unwrap();
    let runner = Arc::new(FakeRunner::with_handler(chain(respond)));
    let installer = WindowsHostInstaller::with_parts(
        Box::new(FakeRunnerHandle(runner.clone())),
        Box::new(FakeSession { interactive: true }),
    );
    Harness {
        state,
        runner,
        installer,
    }
}

fn trust_harness(
    respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
) -> Harness {
    let state = std::env::temp_dir().join(format!("od-win-setup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&state).unwrap();
    let runner = Arc::new(FakeRunner::with_handler(chain(respond)));
    let installer = WindowsHostInstaller::with_parts(
        Box::new(FakeRunnerHandle(runner.clone())),
        Box::new(FakeSession { interactive: true }),
    );
    Harness {
        state,
        runner,
        installer,
    }
}

fn certutil_commands(harness: &Harness) -> Vec<Vec<String>> {
    harness
        .runner
        .calls()
        .iter()
        .filter(|(program, _)| program == "certutil")
        .map(|(_, args)| args.clone())
        .collect()
}

#[test]
fn trust_generates_missing_cert_then_installs_it() {
    let harness = trust_harness(|_, args| {
        if args
            .iter()
            .any(|arg| arg.contains("New-SelfSignedCertificate"))
        {
            Ok("CD34EF56AB78\n".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    harness.installer.apply_setup(&request).unwrap();

    let generated = powershell_commands(&harness);
    assert!(
        generated
            .iter()
            .any(|command| command.contains("New-SelfSignedCertificate")
                && command.contains("CN=OwnDomains Local CA")
                && command.contains("Export-Certificate")),
        "{generated:?}"
    );
    let installed = certutil_commands(&harness);
    assert!(
        installed.iter().any(|args| args
            == &vec![
                "-addstore".to_string(),
                "-user".to_string(),
                "Root".to_string(),
                harness.state.join("ca.crt").display().to_string()
            ]),
        "{installed:?}"
    );
    let receipt: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(harness.state.join("applied").join("trust.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["thumbprint"], "CD34EF56AB78");
    assert_eq!(receipt["generated"], true);
}

#[test]
fn trust_uninstall_removes_generated_cert_file_but_keeps_provided_one() {
    // Generated cert: file deleted on uninstall.
    let generated = trust_harness(|_, args| {
        if args
            .iter()
            .any(|arg| arg.contains("New-SelfSignedCertificate"))
        {
            Ok("CD34EF56AB78\n".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: generated.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    generated.installer.apply_setup(&request).unwrap();
    generated
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: generated.state.clone(),
            purge: false,
        })
        .unwrap();
    let removed = certutil_commands(&generated);
    assert!(
        removed.iter().any(|args| args
            == &vec![
                "-delstore".to_string(),
                "-user".to_string(),
                "Root".to_string(),
                "CD34EF56AB78".to_string()
            ]),
        "{removed:?}"
    );

    // Provided cert: store entry removed, file preserved.
    let provided = firewall_harness(|_, _| Ok(String::new()));
    let request = SetupRequest {
        state_directory: provided.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    provided.installer.apply_setup(&request).unwrap();
    assert!(provided.state.join("ca.crt").is_file());
    provided
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: provided.state.clone(),
            purge: false,
        })
        .unwrap();
    assert!(provided.state.join("ca.crt").is_file());
    let removed = certutil_commands(&provided);
    assert!(
        removed.iter().any(|args| args
            == &vec![
                "-delstore".to_string(),
                "-user".to_string(),
                "Root".to_string(),
                "AB12CD34EF56".to_string()
            ]),
        "{removed:?}"
    );
}

#[test]
fn trust_failure_aborts_without_receipt() {
    let harness = trust_harness(|program, _| {
        if program == "certutil" {
            Err("The request is not supported.".into())
        } else {
            Ok("CD34EF56AB78\n".into())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    let error = harness.installer.apply_setup(&request).unwrap_err();
    assert!(error.message.contains("not supported"), "{}", error.message);
    assert!(!harness.state.join("applied").join("trust.json").exists());
}

fn netsh_calls(harness: &Harness) -> Vec<Vec<String>> {
    harness
        .runner
        .calls()
        .iter()
        .filter(|(program, _)| program == "netsh")
        .map(|(_, args)| args.clone())
        .collect()
}

#[test]
fn firewall_adds_rule_with_exact_argv_when_missing() {
    let harness = firewall_harness(|_, _| Ok(String::new()));
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    harness.installer.apply_setup(&request).unwrap();
    let calls = netsh_calls(&harness);
    let show = calls.iter().position(|args| {
        args == &vec![
            "advfirewall".to_string(),
            "firewall".to_string(),
            "show".to_string(),
            "rule".to_string(),
            "name=OwnDomains-LAN".to_string(),
        ]
    });
    let add = calls.iter().position(|args| {
        args == &vec![
            "advfirewall".to_string(),
            "firewall".to_string(),
            "add".to_string(),
            "rule".to_string(),
            "name=OwnDomains-LAN".to_string(),
            "dir=in".to_string(),
            "action=allow".to_string(),
            "protocol=TCP".to_string(),
            "localport=80,443".to_string(),
        ]
    });
    assert!(show.is_some() && add.is_some(), "{calls:?}");
    assert!(show < add, "{calls:?}");
    let receipt: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(harness.state.join("applied").join("firewall.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["rule"], "OwnDomains-LAN");
}

#[test]
fn firewall_skips_add_when_rule_exists() {
    let harness = firewall_harness(|program, args| {
        if program == "netsh" && args.iter().any(|arg| arg == "show") {
            Ok("Rule Name: OwnDomains-LAN\nEnabled: Yes\n".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    harness.installer.apply_setup(&request).unwrap();
    let calls = netsh_calls(&harness);
    assert!(
        !calls.iter().any(|args| args.contains(&"add".to_string())),
        "{calls:?}"
    );
}

#[test]
fn firewall_add_failure_aborts_without_receipt() {
    let harness = firewall_harness(|program, args| {
        if program == "netsh" && args.iter().any(|arg| arg == "add") {
            Err("Access is denied.".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: harness.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    // NRPT and trust receipts land first (order), firewall fails.
    let error = harness.installer.apply_setup(&request).unwrap_err();
    assert!(
        error.message.contains("Access is denied"),
        "{}",
        error.message
    );
    assert!(harness.state.join("applied").join("nrpt.json").is_file());
    assert!(!harness.state.join("applied").join("firewall.json").exists());
}

#[test]
fn firewall_remove_deletes_present_rule_and_skips_absent() {
    let present = firewall_harness(|program, args| {
        if program == "netsh" && args.iter().any(|arg| arg == "show") {
            Ok("Rule Name: OwnDomains-LAN\n".into())
        } else {
            Ok(String::new())
        }
    });
    let request = SetupRequest {
        state_directory: present.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    present.installer.apply_setup(&request).unwrap();
    present
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: present.state.clone(),
            purge: false,
        })
        .unwrap();
    let calls = netsh_calls(&present);
    assert!(
        calls.iter().any(|args| args
            == &vec![
                "advfirewall".to_string(),
                "firewall".to_string(),
                "delete".to_string(),
                "rule".to_string(),
                "name=OwnDomains-LAN".to_string(),
            ]),
        "{calls:?}"
    );

    let absent = firewall_harness(|_, _| Ok("No rules match the specified criteria.\n".into()));
    let request = SetupRequest {
        state_directory: absent.state.clone(),
        lan_address: "192.168.1.20".into(),
        interface: "Ethernet".into(),
    };
    // Pipe the show output through: apply sees "No rules match" and skips add,
    // so the receipt still lands; uninstall then also skips delete.
    absent.installer.apply_setup(&request).unwrap();
    assert!(absent.state.join("applied").join("firewall.json").is_file());
    absent
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: absent.state.clone(),
            purge: false,
        })
        .unwrap();
    let calls = netsh_calls(&absent);
    assert!(
        !calls
            .iter()
            .any(|args| args.contains(&"delete".to_string())),
        "{calls:?}"
    );
}

#[test]
fn uninstall_purge_removes_the_state_directory() {
    let harness = harness(true);
    let request = setup_request(&harness.state);
    harness.installer.apply_setup(&request).unwrap();
    assert!(harness.state.is_dir());
    harness
        .installer
        .apply_uninstall(&UninstallRequest {
            state_directory: harness.state.clone(),
            purge: true,
        })
        .unwrap();
    assert!(!harness.state.exists());
}
