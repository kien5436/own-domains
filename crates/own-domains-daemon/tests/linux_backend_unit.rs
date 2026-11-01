//! Linux backend path resolution and orchestration units.
//!
//! Host-runnable: pure path logic plus fake adapters. Real sidecars run
//! only under `linux_backend_e2e` on the Ubuntu VM.

use own_domains_daemon::RuntimeBackend;
use own_domains_daemon::platform::linux::backend::{
    BackendAdmin, BackendChild, BackendProcessHost, BackendReadiness, BackendSetup, LinuxAdapters,
    LinuxLiveBackend, bundled_resources_dir, bundled_target, resolve_paths,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type SpawnLog = Arc<Mutex<Vec<(PathBuf, Vec<String>)>>>;
type ProbeLog = Arc<Mutex<Vec<(String, u16, String)>>>;

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("od-linux-backend-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn explicit_sidecars_dir_wins_with_plain_names() {
    let base = tmp_dir("explicit");
    let sidecars = base.join("custom");
    std::fs::create_dir_all(&sidecars).unwrap();

    let paths = resolve_paths(
        Path::new("state"),
        &base,
        &base.join("exe"),
        None,
        Some(sidecars.clone()),
    )
    .unwrap();

    assert_eq!(paths.coredns_exe, sidecars.join("coredns"));
    assert_eq!(paths.caddy_exe, sidecars.join("caddy"));
    assert_eq!(paths.runtime_directory, base.join("state").join("runtime"));
    assert_eq!(
        paths.admin_socket,
        base.join("state").join("runtime").join("caddy-admin.sock")
    );
}

#[test]
fn staged_bundle_uses_triple_infixed_names() {
    let base = tmp_dir("staged");
    let triple = bundled_target();
    let resources = base.join("resources");
    let staged = resources.join("sidecars").join(triple);
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join(format!("coredns-{triple}")), b"fake").unwrap();
    std::fs::write(staged.join(format!("caddy-{triple}")), b"fake").unwrap();

    let paths = resolve_paths(
        &base.join("state"),
        &base,
        &base.join("exe"),
        Some(&resources),
        None,
    )
    .unwrap();

    assert_eq!(paths.coredns_exe, staged.join(format!("coredns-{triple}")));
    assert_eq!(paths.caddy_exe, staged.join(format!("caddy-{triple}")));
}

#[test]
fn falls_back_to_exe_dir_without_bundle() {
    let base = tmp_dir("fallback");
    let exe = base.join("exe");
    std::fs::create_dir_all(&exe).unwrap();

    let paths = resolve_paths(&base.join("state"), &base, &exe, None, None).unwrap();

    assert_eq!(paths.coredns_exe, exe.join("coredns"));
    assert_eq!(paths.caddy_exe, exe.join("caddy"));
}

#[test]
fn bundled_resources_dir_detects_sidecars_layout() {
    let base = tmp_dir("resources");
    let exe = base.join("exe");
    std::fs::create_dir_all(exe.join("sidecars")).unwrap();
    assert_eq!(bundled_resources_dir(&exe), Some(exe.clone()));

    let bare = base.join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    assert_eq!(bundled_resources_dir(&bare), None);
}

struct FakeChild {
    terminated: Arc<Mutex<u32>>,
}

impl BackendChild for FakeChild {
    fn terminate(&self) -> Result<(), String> {
        *self.terminated.lock().unwrap() += 1;
        Ok(())
    }
    fn wait_exit(&self, _timeout: std::time::Duration) -> Result<u32, String> {
        Ok(0)
    }
    fn try_wait(&self) -> Result<Option<u32>, String> {
        Ok(None)
    }
}

struct FakeHost {
    spawns: SpawnLog,
    terminated: Arc<Mutex<u32>>,
}

impl BackendProcessHost for FakeHost {
    fn spawn(&self, exe: &Path, args: &[String]) -> Result<Box<dyn BackendChild>, String> {
        self.spawns
            .lock()
            .unwrap()
            .push((exe.to_path_buf(), args.to_vec()));
        Ok(Box::new(FakeChild {
            terminated: self.terminated.clone(),
        }))
    }
}

struct FakeReadiness {
    dns: ProbeLog,
    http: ProbeLog,
    fail_dns: bool,
}

impl BackendReadiness for FakeReadiness {
    fn wait_dns(&self, address: &str, port: u16, expected: &str) -> Result<(), String> {
        self.dns
            .lock()
            .unwrap()
            .push((address.into(), port, expected.into()));
        if self.fail_dns {
            return Err("dns not ready".into());
        }
        Ok(())
    }
    fn wait_http(&self, address: &str, port: u16, host: &str) -> Result<(), String> {
        self.http
            .lock()
            .unwrap()
            .push((address.into(), port, host.into()));
        Ok(())
    }
}

struct FakeAdmin {
    loads: Arc<Mutex<u32>>,
}

impl BackendAdmin for FakeAdmin {
    fn load(&self, _config: &serde_json::Value) -> Result<(), String> {
        *self.loads.lock().unwrap() += 1;
        Ok(())
    }
}

struct FakeSetup {
    running: Arc<Mutex<bool>>,
    starts: Arc<Mutex<Vec<std::net::SocketAddr>>>,
}

impl BackendSetup for FakeSetup {
    fn start(&self, address: std::net::SocketAddr, _certificate: PathBuf) -> Result<(), String> {
        self.starts.lock().unwrap().push(address);
        *self.running.lock().unwrap() = true;
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        *self.running.lock().unwrap() = false;
        Ok(())
    }
    fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }
}

struct Fakes {
    host: FakeHost,
    readiness: FakeReadiness,
    admin: FakeAdmin,
    setup: FakeSetup,
    spawns: SpawnLog,
    terminated: Arc<Mutex<u32>>,
    dns: ProbeLog,
    http: ProbeLog,
    loads: Arc<Mutex<u32>>,
    running: Arc<Mutex<bool>>,
    starts: Arc<Mutex<Vec<std::net::SocketAddr>>>,
}

fn fakes(fail_dns: bool) -> Fakes {
    let spawns = Arc::new(Mutex::new(Vec::new()));
    let terminated = Arc::new(Mutex::new(0u32));
    let dns = Arc::new(Mutex::new(Vec::new()));
    let http = Arc::new(Mutex::new(Vec::new()));
    let loads = Arc::new(Mutex::new(0u32));
    let running = Arc::new(Mutex::new(false));
    let starts = Arc::new(Mutex::new(Vec::new()));
    Fakes {
        host: FakeHost {
            spawns: spawns.clone(),
            terminated: terminated.clone(),
        },
        readiness: FakeReadiness {
            dns: dns.clone(),
            http: http.clone(),
            fail_dns,
        },
        admin: FakeAdmin {
            loads: loads.clone(),
        },
        setup: FakeSetup {
            running: running.clone(),
            starts: starts.clone(),
        },
        spawns,
        terminated,
        dns,
        http,
        loads,
        running,
        starts,
    }
}

fn test_settings() -> own_domains_core::Settings {
    own_domains_core::Settings {
        schema_version: 1,
        revision: 7,
        lan_address: "192.168.1.21".into(),
        upstream_dns: vec!["1.1.1.1".into()],
        mappings: vec![own_domains_core::DomainMapping {
            id: "app".into(),
            hostname: "app.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        }],
        extra: Default::default(),
    }
}

fn live_backend<'a>(tag: &str, fakes: &'a Fakes) -> LinuxLiveBackend<'a> {
    let base = tmp_dir(tag);
    let sidecars = base.join("sidecars");
    std::fs::create_dir_all(&sidecars).unwrap();
    let state = base.join("state");
    let paths = resolve_paths(&state, &base, &base.join("exe"), None, Some(sidecars)).unwrap();
    LinuxLiveBackend::with_adapters(
        paths,
        LinuxAdapters {
            process_host: &fakes.host,
            readiness: &fakes.readiness,
            caddy: &fakes.admin,
            setup: &fakes.setup,
        },
    )
}

#[test]
fn apply_spawns_probes_loads_and_starts_setup() {
    let fakes = fakes(false);
    let backend = live_backend("apply", &fakes);
    let settings = test_settings();

    backend.apply(&settings).unwrap();

    let spawns = fakes.spawns.lock().unwrap();
    assert_eq!(spawns.len(), 2);
    assert!(spawns[0].0.ends_with("coredns"));
    assert_eq!(spawns[0].1[0], "-conf");
    assert!(spawns[0].1[1].ends_with("Corefile"));
    assert!(spawns[1].0.ends_with("caddy"));
    assert_eq!(spawns[1].1[0], "run");

    let dns = fakes.dns.lock().unwrap();
    assert_eq!(
        dns.as_slice(),
        &[("127.0.0.1".to_string(), 5354, "192.168.1.21".to_string())]
    );
    let http = fakes.http.lock().unwrap();
    assert_eq!(
        http.as_slice(),
        &[("127.0.0.1".to_string(), 8080, "app.own".to_string())]
    );
    assert_eq!(*fakes.loads.lock().unwrap(), 1);
    assert!(*fakes.running.lock().unwrap());
    let starts = fakes.starts.lock().unwrap();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].port(), 4781);
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Running
    ));
}

#[test]
fn reapply_at_same_revision_is_noop() {
    let fakes = fakes(false);
    let backend = live_backend("reapply", &fakes);
    let settings = test_settings();

    backend.apply(&settings).unwrap();
    backend.apply(&settings).unwrap();

    assert_eq!(fakes.spawns.lock().unwrap().len(), 2);
    assert_eq!(*fakes.loads.lock().unwrap(), 1);
}

#[test]
fn stop_terminates_children_and_reports_stopped() {
    let fakes = fakes(false);
    let backend = live_backend("stop", &fakes);
    let settings = test_settings();

    backend.apply(&settings).unwrap();
    backend.stop().unwrap();

    assert_eq!(*fakes.terminated.lock().unwrap(), 2);
    assert!(!*fakes.running.lock().unwrap());
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Stopped
    ));
}

#[test]
fn apply_failure_terminates_spawned_children_and_stops_setup() {
    let fakes = fakes(true);
    let backend = live_backend("fail", &fakes);
    let settings = test_settings();

    assert!(backend.apply(&settings).is_err());

    assert_eq!(*fakes.terminated.lock().unwrap(), 2);
    assert!(!*fakes.running.lock().unwrap());
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Stopped
    ));
}

#[cfg(unix)]
#[test]
fn system_process_host_spawns_and_reaps() {
    use own_domains_daemon::platform::linux::backend::SystemProcessHost;
    use std::time::Duration;

    let host = SystemProcessHost;
    let child = host.spawn(Path::new("/usr/bin/true"), &[]).unwrap();
    assert_eq!(child.wait_exit(Duration::from_secs(5)).unwrap(), 0);

    let child = host
        .spawn(Path::new("/bin/sleep"), &["60".to_string()])
        .unwrap();
    child.terminate().unwrap();
    child.wait_exit(Duration::from_secs(5)).unwrap();
}

#[test]
fn system_setup_starts_stops_and_is_idempotent() {
    use own_domains_daemon::platform::linux::backend::SystemSetup;

    let setup = SystemSetup::new();
    assert!(!setup.is_running());
    let cert = tmp_dir("cert").join("ca.crt");
    setup
        .start(std::net::SocketAddr::from(([127, 0, 0, 1], 0)), cert)
        .unwrap();
    assert!(setup.is_running());
    let cert2 = tmp_dir("cert2").join("ca.crt");
    setup
        .start(std::net::SocketAddr::from(([127, 0, 0, 1], 0)), cert2)
        .unwrap();
    assert!(setup.is_running());
    setup.stop().unwrap();
    assert!(!setup.is_running());
}

#[test]
fn factory_returns_unavailable_for_settings_only() {
    use own_domains_daemon::platform::linux::backend::runtime_backend;

    let dir = tmp_dir("settings-only");
    let backend = runtime_backend(&dir, None, None, true).unwrap();
    assert!(
        backend
            .diagnose(&own_domains_core::Settings::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn factory_reports_setup_required_without_manifest() {
    use own_domains_daemon::platform::linux::backend::runtime_backend;

    let dir = tmp_dir("facade");
    let backend = runtime_backend(&dir, None, None, false).unwrap();
    let diagnostics = backend
        .diagnose(&own_domains_core::Settings::default())
        .unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "setupRequired");
}
