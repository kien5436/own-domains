//! Windows runtime backend skeleton, tested without real sidecars.
//!
//! Fake adapters record every effect; real DNS/HTTP/Caddy behavior is covered
//! on the VM by `windows_backend.rs`.

use own_domains_core::{ApplyStatus, DomainMapping, Settings};
use own_domains_daemon::{
    RuntimeBackend,
    platform::windows::backend::{
        BackendAdmin, BackendChild, BackendProcessHost, BackendReadiness, WindowsAdapters,
        WindowsPaths, WindowsRuntimeBackend,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Log {
    events: Mutex<Vec<String>>,
}

impl Log {
    fn push(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }
}

struct FakeChild {
    log: Arc<Log>,
    name: String,
}

impl BackendChild for FakeChild {
    fn terminate(&self) -> Result<(), String> {
        self.log.push(format!("terminate {}", self.name));
        Ok(())
    }

    fn wait_exit(&self, _timeout: Duration) -> Result<u32, String> {
        Ok(0)
    }
}

struct FakeHost {
    log: Arc<Log>,
}

impl BackendProcessHost for FakeHost {
    fn spawn(&self, exe: &Path, args: &[String]) -> Result<Box<dyn BackendChild>, String> {
        let name = exe
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.log.push(format!("spawn {name} {}", args.join(" ")));
        Ok(Box::new(FakeChild {
            log: self.log.clone(),
            name,
        }))
    }
}

struct FakeReadiness {
    log: Arc<Log>,
}

impl BackendReadiness for FakeReadiness {
    fn wait_dns(&self, address: &str, port: u16, expected: &str) -> Result<(), String> {
        self.log
            .push(format!("wait-dns {address}:{port}={expected}"));
        Ok(())
    }

    fn wait_http(&self, address: &str, port: u16, host: &str) -> Result<(), String> {
        self.log
            .push(format!("wait-http {address}:{port} host={host}"));
        Ok(())
    }
}

struct FakeAdmin {
    log: Arc<Log>,
}

impl BackendAdmin for FakeAdmin {
    fn load(&self, _config: &serde_json::Value) -> Result<(), String> {
        self.log.push("admin-load".to_string());
        Ok(())
    }
}

fn settings() -> Settings {
    Settings {
        lan_address: "192.168.1.20".into(),
        upstream_dns: vec!["1.1.1.1".into()],
        revision: 5,
        mappings: vec![DomainMapping {
            id: "shop".into(),
            hostname: "shop.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        }],
        ..Settings::default()
    }
}

fn backend(state: &BackendHarness) -> WindowsRuntimeBackend<'_> {
    WindowsRuntimeBackend::with_adapters(
        WindowsPaths {
            runtime_directory: state.directory.clone(),
            coredns_exe: PathBuf::from("coredns-test.exe"),
            caddy_exe: PathBuf::from("caddy-test.exe"),
            admin_socket: state.directory.join("caddy-admin.sock"),
        },
        WindowsAdapters {
            process_host: &state.host,
            readiness: &state.readiness,
            caddy: &state.admin,
        },
    )
}

struct BackendHarness {
    directory: PathBuf,
    log: Arc<Log>,
    host: FakeHost,
    readiness: FakeReadiness,
    admin: FakeAdmin,
}

fn harness() -> BackendHarness {
    let directory = std::env::temp_dir().join(format!("od-win-backend-{}", uuid::Uuid::new_v4()));
    let log = Arc::new(Log::default());
    BackendHarness {
        directory,
        log: log.clone(),
        host: FakeHost { log: log.clone() },
        readiness: FakeReadiness { log: log.clone() },
        admin: FakeAdmin { log },
    }
}

#[test]
fn apply_writes_configs_and_starts_sidecars() {
    let harness = harness();
    let backend = backend(&harness);
    let result = backend.apply(&settings()).unwrap();
    assert_eq!(result.applied_revision, 5);
    assert!(matches!(result.status, ApplyStatus::Applied));

    let corefile = std::fs::read_to_string(harness.directory.join("Corefile")).unwrap();
    assert!(corefile.contains("0 IN A 192.168.1.20"), "{corefile}");
    let proxy = std::fs::read_to_string(harness.directory.join("caddy.json")).unwrap();
    let proxy_json: serde_json::Value = serde_json::from_str(&proxy).unwrap();
    for server in ["http", "https"] {
        assert_eq!(
            proxy_json["apps"]["http"]["servers"][server]["protocols"],
            serde_json::json!(["h1", "h2"]),
            "{server}"
        );
    }

    let log = harness.log.events.lock().unwrap().join("\n");
    assert!(log.contains("spawn coredns-test.exe"), "{log}");
    assert!(log.contains("spawn caddy-test.exe"), "{log}");
    assert!(log.contains("wait-dns"), "{log}");
    assert!(log.contains("wait-http"), "{log}");
    assert!(log.contains("admin-load"), "{log}");
}

#[test]
fn stop_terminates_processes_and_is_idempotent() {
    let harness = harness();
    let backend = backend(&harness);
    backend.apply(&settings()).unwrap();
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Running
    ));
    backend.stop().unwrap();
    let log = harness.log.events.lock().unwrap().join("\n");
    assert!(log.contains("terminate coredns-test.exe"), "{log}");
    assert!(log.contains("terminate caddy-test.exe"), "{log}");
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Stopped
    ));
    backend.stop().unwrap();
}
