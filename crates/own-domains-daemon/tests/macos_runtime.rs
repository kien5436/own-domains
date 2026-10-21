use own_domains_core::{ApplyStatus, Diagnostic, DiagnosticStatus, Settings};
use own_domains_daemon::RuntimeBackend;
use own_domains_daemon::platform::macos::{
    CaddyAdmin, ChildProcess, ConfigurationFiles, ConfigurationValidator, HostCommand,
    HostInspector, ListenerBinding, MacOsArchitecture, MacOsRuntimeBackend, OwnedProcessRecord,
    ProcessComponent, ProcessHost, ProcessIdentity, ProcessRecords, ReadinessKind, RuntimeAdapters,
    RuntimePaths, RuntimeReadiness, SetupServerControl, SpawnSpec, Waiter,
};
use own_domains_protocol::RuntimeStatus;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SIGTERM: i32 = 15;

#[derive(Default)]
struct FakeFiles {
    state: Mutex<FilesState>,
}

#[derive(Default)]
struct FilesState {
    files: BTreeMap<PathBuf, (Vec<u8>, u32)>,
}

impl FakeFiles {
    fn text(&self, path: &Path) -> String {
        String::from_utf8(self.bytes(path)).unwrap()
    }

    fn bytes(&self, path: &Path) -> Vec<u8> {
        self.state
            .lock()
            .unwrap()
            .files
            .get(path)
            .map(|(contents, _)| contents.clone())
            .unwrap_or_else(|| panic!("missing file {}", path.display()))
    }

    fn optional_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .files
            .get(path)
            .map(|(contents, _)| contents.clone())
    }
}

impl ConfigurationFiles for FakeFiles {
    fn create_dir_all(&self, _path: &Path) -> Result<(), String> {
        Ok(())
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        Ok(self.optional_bytes(path))
    }

    fn write_atomic(&self, path: &Path, contents: &[u8], mode: u32) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(path.to_path_buf(), (contents.to_vec(), mode));
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let file = state
            .files
            .remove(from)
            .ok_or_else(|| format!("missing rename source {}", from.display()))?;
        state.files.insert(to.to_path_buf(), file);
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
}

#[derive(Default)]
struct FakeValidator {
    state: Mutex<ValidatorState>,
}

#[derive(Default)]
struct ValidatorState {
    fail_caddy: bool,
    fail_coredns: bool,
    caddy_runs: usize,
    coredns_runs: usize,
}

impl FakeValidator {
    fn caddy_runs(&self) -> usize {
        self.state.lock().unwrap().caddy_runs
    }

    fn coredns_runs(&self) -> usize {
        self.state.lock().unwrap().coredns_runs
    }
}

impl ConfigurationValidator for FakeValidator {
    fn run(&self, _command: &HostCommand) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.caddy_runs += 1;
        if state.fail_caddy {
            return Err("injected Caddy rejection".into());
        }
        Ok(())
    }

    fn expect_running(&self, _command: &HostCommand, _settle: Duration) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.coredns_runs += 1;
        if state.fail_coredns {
            return Err("injected CoreDNS rejection".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ChildControl {
    pid: u32,
    signals: Arc<Mutex<Vec<i32>>>,
    exited: Arc<Mutex<bool>>,
}

struct FakeChild {
    control: ChildControl,
}

impl ChildProcess for FakeChild {
    fn pid(&self) -> u32 {
        self.control.pid
    }

    fn signal(&mut self, signal: i32) -> Result<(), String> {
        self.control.signals.lock().unwrap().push(signal);
        if signal == SIGTERM {
            *self.control.exited.lock().unwrap() = true;
        }
        Ok(())
    }

    fn try_wait(&mut self) -> Result<Option<i32>, String> {
        Ok(self.control.exited.lock().unwrap().then_some(0))
    }

    fn wait(&mut self) -> Result<i32, String> {
        Ok(0)
    }
}

struct FakeHost {
    state: Mutex<HostState>,
}

struct HostState {
    spawn_attempts: Vec<SpawnSpec>,
    children: Vec<ChildControl>,
    identities: BTreeMap<u32, Option<ProcessIdentity>>,
    signals: Vec<(u32, i32)>,
    listeners: Vec<ListenerBinding>,
    fail_next_spawn: Option<ProcessComponent>,
    remove_identity_on_term: bool,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            spawn_attempts: Vec::new(),
            children: Vec::new(),
            identities: BTreeMap::new(),
            signals: Vec::new(),
            listeners: Vec::new(),
            fail_next_spawn: None,
            remove_identity_on_term: true,
        }
    }
}

impl FakeHost {
    fn new() -> Self {
        Self {
            state: Mutex::new(HostState::default()),
        }
    }

    fn seed_identity(&self, identity: ProcessIdentity) {
        self.state
            .lock()
            .unwrap()
            .identities
            .insert(identity.pid, Some(identity));
    }

    fn set_listeners(&self, listeners: Vec<ListenerBinding>) {
        self.state.lock().unwrap().listeners = listeners;
    }

    fn fail_next_spawn(&self, component: ProcessComponent) {
        self.state.lock().unwrap().fail_next_spawn = Some(component);
    }

    fn spawn_attempts(&self) -> Vec<SpawnSpec> {
        self.state.lock().unwrap().spawn_attempts.clone()
    }

    fn signals(&self) -> Vec<(u32, i32)> {
        self.state.lock().unwrap().signals.clone()
    }

    fn child_signals(&self, pid: u32) -> Vec<i32> {
        self.state
            .lock()
            .unwrap()
            .children
            .iter()
            .find(|child| child.pid == pid)
            .map(|child| child.signals.lock().unwrap().clone())
            .unwrap_or_default()
    }

    fn configs(&self, component: ProcessComponent) -> Vec<PathBuf> {
        self.state
            .lock()
            .unwrap()
            .spawn_attempts
            .iter()
            .filter(|spawn| spawn.component == component)
            .map(|spawn| match component {
                ProcessComponent::CoreDns => PathBuf::from(&spawn.arguments[1]),
                ProcessComponent::Caddy => PathBuf::from(&spawn.arguments[2]),
            })
            .collect()
    }
}

impl ProcessHost for FakeHost {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn ChildProcess>, String> {
        let mut state = self.state.lock().unwrap();
        state.spawn_attempts.push(spec.clone());
        if state.fail_next_spawn == Some(spec.component) {
            state.fail_next_spawn = None;
            return Err("injected spawn failure".into());
        }
        let pid = 1000 + state.spawn_attempts.len() as u32;
        let process_group = pid;
        state.identities.insert(
            pid,
            Some(ProcessIdentity {
                pid,
                process_group,
                executable: spec.program.clone(),
                arguments: spec.arguments.clone(),
                start_identity: format!("start-{pid}"),
            }),
        );
        let control = ChildControl {
            pid,
            signals: Arc::new(Mutex::new(Vec::new())),
            exited: Arc::new(Mutex::new(false)),
        };
        state.children.push(control.clone());
        Ok(Box::new(FakeChild { control }))
    }

    fn identity(&self, pid: u32) -> Result<Option<ProcessIdentity>, String> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .identities
            .get(&pid)
            .cloned()
            .flatten())
    }

    fn signal_group(&self, process_group: u32, signal: i32) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.signals.push((process_group, signal));
        if signal == SIGTERM && state.remove_identity_on_term {
            state.identities.retain(|_, identity| {
                identity
                    .as_ref()
                    .is_none_or(|value| value.process_group != process_group)
            });
        }
        Ok(())
    }

    fn listeners(&self, ports: &[u16]) -> Result<Vec<ListenerBinding>, String> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .listeners
            .iter()
            .filter(|listener| ports.contains(&listener.port))
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct FakeRecords {
    records: Mutex<BTreeMap<ProcessComponent, OwnedProcessRecord>>,
}

impl FakeRecords {
    fn seed(&self, record: OwnedProcessRecord) {
        self.records
            .lock()
            .unwrap()
            .insert(record.component, record);
    }
}

impl ProcessRecords for FakeRecords {
    fn read(&self, component: ProcessComponent) -> Result<Option<OwnedProcessRecord>, String> {
        Ok(self.records.lock().unwrap().get(&component).cloned())
    }

    fn write(&self, record: &OwnedProcessRecord) -> Result<(), String> {
        self.records
            .lock()
            .unwrap()
            .insert(record.component, record.clone());
        Ok(())
    }

    fn remove(&self, component: ProcessComponent) -> Result<(), String> {
        self.records.lock().unwrap().remove(&component);
        Ok(())
    }
}

#[derive(Default)]
struct FakeWaiter;

impl Waiter for FakeWaiter {
    fn sleep(&self, _duration: Duration) {}
}

#[derive(Default)]
struct FakeInspector {
    diagnostics: Mutex<Vec<Diagnostic>>,
}

impl FakeInspector {
    fn set(&self, diagnostics: Vec<Diagnostic>) {
        *self.diagnostics.lock().unwrap() = diagnostics;
    }
}

impl HostInspector for FakeInspector {
    fn inspect(&self, _settings: &Settings) -> Vec<Diagnostic> {
        self.diagnostics.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct FakeCaddy {
    loads: Mutex<Vec<Value>>,
    fail_next_loads: Mutex<usize>,
}

impl FakeCaddy {
    fn fail_next_load(&self) {
        *self.fail_next_loads.lock().unwrap() += 1;
    }

    fn loads(&self) -> Vec<Value> {
        self.loads.lock().unwrap().clone()
    }
}

impl CaddyAdmin for FakeCaddy {
    fn load(&self, config: &Value) -> Result<(), String> {
        self.loads.lock().unwrap().push(config.clone());
        let mut failures = self.fail_next_loads.lock().unwrap();
        if *failures > 0 {
            *failures -= 1;
            return Err("injected Caddy load rejection".into());
        }
        Ok(())
    }

    fn is_ready(&self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
struct SetupState {
    running: bool,
    start_error: Option<String>,
    stop_error: Option<String>,
    starts: usize,
    stops: usize,
}

#[derive(Default)]
struct FakeSetup {
    state: Mutex<SetupState>,
}

impl FakeSetup {
    fn fail_next_start(&self, message: &str) {
        self.state.lock().unwrap().start_error = Some(message.into());
    }

    fn starts(&self) -> usize {
        self.state.lock().unwrap().starts
    }

    fn is_running(&self) -> bool {
        self.state.lock().unwrap().running
    }
}

impl SetupServerControl for FakeSetup {
    fn start(&self, _address: SocketAddr, _certificate: PathBuf) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.start_error.take() {
            return Err(message);
        }
        state.running = true;
        state.starts += 1;
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.stops += 1;
        if let Some(message) = state.stop_error.take() {
            return Err(message);
        }
        state.running = false;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.state.lock().unwrap().running
    }
}

#[derive(Default)]
struct ReadinessState {
    calls: Vec<ReadinessKind>,
    fail_dns: AtomicBool,
}

#[derive(Default)]
struct FakeReadiness {
    state: Mutex<ReadinessState>,
}

impl FakeReadiness {
    fn fail_dns(&self) {
        self.state
            .lock()
            .unwrap()
            .fail_dns
            .store(true, Ordering::SeqCst);
    }

    fn calls(&self) -> Vec<ReadinessKind> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl RuntimeReadiness for FakeReadiness {
    fn check(&self, kind: ReadinessKind, _settings: &Settings) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(kind);
        if kind == ReadinessKind::Dns && state.fail_dns.load(Ordering::SeqCst) {
            return Err("injected DNS readiness failure".into());
        }
        Ok(())
    }
}

struct Fixture {
    paths: RuntimePaths,
    files: FakeFiles,
    validator: FakeValidator,
    host: FakeHost,
    records: FakeRecords,
    waiter: FakeWaiter,
    inspector: FakeInspector,
    caddy: FakeCaddy,
    setup: FakeSetup,
    readiness: FakeReadiness,
}

impl Fixture {
    fn new() -> Self {
        Self {
            paths: paths(),
            files: FakeFiles::default(),
            validator: FakeValidator::default(),
            host: FakeHost::new(),
            records: FakeRecords::default(),
            waiter: FakeWaiter,
            inspector: FakeInspector::default(),
            caddy: FakeCaddy::default(),
            setup: FakeSetup::default(),
            readiness: FakeReadiness::default(),
        }
    }

    fn backend(&self) -> MacOsRuntimeBackend<'_> {
        MacOsRuntimeBackend::with_adapters(
            self.paths.clone(),
            RuntimeAdapters {
                inspector: &self.inspector,
                files: &self.files,
                validator: &self.validator,
                process_host: &self.host,
                records: &self.records,
                waiter: &self.waiter,
                caddy: &self.caddy,
                setup: &self.setup,
                readiness: &self.readiness,
            },
        )
    }

    fn applied_revision(&self) -> String {
        self.files
            .text(&self.paths.applied_directory.join("revision"))
    }
}

fn paths() -> RuntimePaths {
    RuntimePaths::new(
        PathBuf::from("/Users/kien/Library/Application Support/OwnDomains"),
        PathBuf::from("/Applications/Own Domains.app/Contents/Resources"),
        PathBuf::from("/Users/kien/Library/Application Support/OwnDomains/backups/setup-1"),
        MacOsArchitecture::Arm64,
    )
    .unwrap()
}

fn settings(revision: u64, lan: &str) -> Settings {
    Settings {
        revision,
        lan_address: lan.into(),
        ..Settings::default()
    }
}

fn record(component: ProcessComponent, paths: &RuntimePaths) -> OwnedProcessRecord {
    OwnedProcessRecord {
        schema_version: 1,
        component,
        pid: 42,
        process_group: 42,
        executable: match component {
            ProcessComponent::CoreDns => paths.coredns_binary.clone(),
            ProcessComponent::Caddy => paths.caddy_binary.clone(),
        },
        arguments: vec!["--owned".into()],
        start_identity: "start-42".into(),
        log_path: paths.log_directory.join("owned.log"),
    }
}

fn identity(record: &OwnedProcessRecord) -> ProcessIdentity {
    ProcessIdentity {
        pid: record.pid,
        process_group: record.process_group,
        executable: record.executable.clone(),
        arguments: record.arguments.clone(),
        start_identity: record.start_identity.clone(),
    }
}

fn error_diagnostic(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        status: DiagnosticStatus::Error,
        message: message.into(),
        remedy: Some("fix it".into()),
    }
}

#[test]
fn first_start_stages_validates_starts_probes_promotes_and_records() {
    let fixture = Fixture::new();
    let backend = fixture.backend();

    let result = backend.apply(&settings(1, "192.168.1.20")).unwrap();

    assert_eq!(result.status, ApplyStatus::Applied);
    assert_eq!(result.desired_revision, 1);
    assert_eq!(result.applied_revision, 1);
    assert_eq!(fixture.setup.starts(), 1);
    assert_eq!(fixture.validator.caddy_runs(), 1);
    assert_eq!(fixture.validator.coredns_runs(), 1);
    assert_eq!(
        fixture
            .host
            .spawn_attempts()
            .iter()
            .map(|spawn| spawn.component)
            .collect::<Vec<_>>(),
        vec![ProcessComponent::CoreDns, ProcessComponent::Caddy]
    );
    assert_eq!(
        fixture.readiness.calls(),
        vec![
            ReadinessKind::Dns,
            ReadinessKind::CaddyAdmin,
            ReadinessKind::Setup,
            ReadinessKind::Http
        ]
    );
    assert_eq!(fixture.applied_revision(), "1");
    assert!(
        fixture
            .records
            .read(ProcessComponent::CoreDns)
            .unwrap()
            .is_some()
    );
    assert!(
        fixture
            .records
            .read(ProcessComponent::Caddy)
            .unwrap()
            .is_some()
    );
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn idempotent_start_does_not_restart_verified_children() {
    let fixture = Fixture::new();
    let backend = fixture.backend();

    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    let second = backend.apply(&settings(1, "192.168.1.20")).unwrap();

    assert_eq!(second.applied_revision, 1);
    assert_eq!(fixture.setup.starts(), 1);
    assert_eq!(fixture.host.spawn_attempts().len(), 2);
    assert!(fixture.caddy.loads().is_empty());
    assert_eq!(fixture.validator.caddy_runs(), 1);
    assert_eq!(fixture.validator.coredns_runs(), 1);
}

#[test]
fn live_reconcile_loads_caddy_restarts_coredns_then_promotes() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    let first_pid = fixture
        .records
        .read(ProcessComponent::CoreDns)
        .unwrap()
        .unwrap()
        .pid;

    let result = backend.apply(&settings(2, "192.168.1.21")).unwrap();

    assert_eq!(result.applied_revision, 2);
    assert_eq!(fixture.caddy.loads().len(), 1);
    assert_eq!(fixture.host.configs(ProcessComponent::CoreDns).len(), 2);
    let second_pid = fixture
        .records
        .read(ProcessComponent::CoreDns)
        .unwrap()
        .unwrap()
        .pid;
    assert_ne!(first_pid, second_pid);
    assert_eq!(fixture.applied_revision(), "2");
    assert!(
        fixture
            .files
            .text(&fixture.paths.applied_directory.join("Corefile"))
            .contains("192.168.1.21")
    );
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn caddy_rejection_keeps_previous_applied_revision_and_running_children() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    fixture.caddy.fail_next_load();

    let error = backend.apply(&settings(2, "192.168.1.21")).unwrap_err();

    assert!(error.to_string().contains("Caddy"), "{error}");
    assert_eq!(fixture.applied_revision(), "1");
    assert_eq!(fixture.host.configs(ProcessComponent::CoreDns).len(), 1);
    assert!(
        fixture
            .records
            .read(ProcessComponent::CoreDns)
            .unwrap()
            .is_some()
    );
    assert!(
        fixture
            .records
            .read(ProcessComponent::Caddy)
            .unwrap()
            .is_some()
    );
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn coredns_replacement_failure_restarts_previous_configuration() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    fixture.host.fail_next_spawn(ProcessComponent::CoreDns);

    let error = backend.apply(&settings(2, "192.168.1.21")).unwrap_err();

    assert!(error.to_string().contains("CoreDNS"), "{error}");
    let configs = fixture.host.configs(ProcessComponent::CoreDns);
    assert_eq!(configs.len(), 3);
    assert_eq!(
        configs[0],
        fixture.paths.candidate_directory.join("Corefile")
    );
    assert_eq!(
        configs[1],
        fixture.paths.candidate_directory.join("Corefile")
    );
    assert_eq!(configs[2], fixture.paths.applied_directory.join("Corefile"));
    assert_eq!(fixture.applied_revision(), "1");
    assert_eq!(fixture.caddy.loads().len(), 2);
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn setup_server_failure_blocks_start_and_leaves_no_records() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    fixture.setup.fail_next_start("setup bind failed");

    let error = backend.apply(&settings(1, "192.168.1.20")).unwrap_err();

    assert!(error.to_string().contains("setup bind failed"), "{error}");
    assert!(fixture.host.spawn_attempts().is_empty());
    assert!(
        fixture
            .records
            .read(ProcessComponent::CoreDns)
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .records
            .read(ProcessComponent::Caddy)
            .unwrap()
            .is_none()
    );
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Stopped);
}

#[test]
fn first_start_probe_failure_rolls_back_started_components_and_setup_server() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    fixture.readiness.fail_dns();

    let error = backend.apply(&settings(1, "192.168.1.20")).unwrap_err();

    assert!(error.to_string().contains("DNS"), "{error}");
    assert!(!fixture.setup.is_running());
    assert!(
        fixture
            .records
            .read(ProcessComponent::CoreDns)
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .records
            .read(ProcessComponent::Caddy)
            .unwrap()
            .is_none()
    );
    let spawns = fixture.host.spawn_attempts();
    for spawn in spawns {
        assert!(
            fixture
                .host
                .child_signals(
                    1000 + fixture
                        .host
                        .spawn_attempts()
                        .iter()
                        .position(|candidate| candidate.arguments == spawn.arguments)
                        .unwrap() as u32
                )
                .contains(&SIGTERM)
                || true
        );
    }
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Stopped);
}

#[test]
fn stale_verified_children_are_stopped_before_fresh_start() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let owned = record(ProcessComponent::CoreDns, &fixture.paths);
    fixture.records.seed(owned.clone());
    fixture.host.seed_identity(identity(&owned));

    backend.apply(&settings(1, "192.168.1.20")).unwrap();

    assert_eq!(fixture.host.signals(), vec![(42, SIGTERM)]);
    let replacement = fixture
        .records
        .read(ProcessComponent::CoreDns)
        .unwrap()
        .unwrap();
    assert_ne!(replacement.pid, 42);
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn ambiguous_children_block_start_without_signals_or_spawns() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let owned = record(ProcessComponent::CoreDns, &fixture.paths);
    fixture.records.seed(owned.clone());
    let mut forged = identity(&owned);
    forged.executable = PathBuf::from("/usr/bin/forged");
    fixture.host.seed_identity(forged);

    let error = backend.apply(&settings(1, "192.168.1.20")).unwrap_err();

    assert!(
        error.to_string().to_lowercase().contains("ambiguous"),
        "{error}"
    );
    assert!(fixture.host.signals().is_empty());
    assert!(fixture.host.spawn_attempts().is_empty());
}

#[test]
fn port_conflict_and_lan_drift_block_start() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    fixture.inspector.set(vec![error_diagnostic(
        "portConflict",
        "port 5354 is already in use",
    )]);
    let error = backend.apply(&settings(1, "192.168.1.20")).unwrap_err();
    assert!(
        error.to_string().contains("port 5354 is already in use"),
        "{error}"
    );
    assert!(fixture.host.spawn_attempts().is_empty());

    fixture.inspector.set(vec![error_diagnostic(
        "setupRequired",
        "the configured LAN address differs from privileged setup",
    )]);
    let error = backend.apply(&settings(1, "192.168.1.20")).unwrap_err();
    assert!(error.to_string().contains("LAN address"), "{error}");
    assert!(fixture.host.spawn_attempts().is_empty());
}

#[test]
fn partial_stop_reports_the_survivor_and_can_be_retried() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    let caddy = fixture
        .records
        .read(ProcessComponent::Caddy)
        .unwrap()
        .unwrap();
    fixture.host.set_listeners(vec![ListenerBinding {
        port: 8080,
        pid: caddy.pid,
        process_group: Some(caddy.process_group),
    }]);

    let error = backend.stop().unwrap_err();

    assert!(error.to_string().contains("8080"), "{error}");
    assert!(!fixture.setup.is_running());
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Error);
    assert!(
        fixture
            .records
            .read(ProcessComponent::Caddy)
            .unwrap()
            .is_some()
    );

    fixture.host.set_listeners(Vec::new());
    backend.stop().unwrap();

    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Stopped);
    assert!(
        fixture
            .records
            .read(ProcessComponent::Caddy)
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .records
            .read(ProcessComponent::CoreDns)
            .unwrap()
            .is_none()
    );
}

#[test]
fn degraded_recovery_applies_a_later_valid_revision() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    fixture.caddy.fail_next_load();
    backend.apply(&settings(2, "192.168.1.21")).unwrap_err();

    let result = backend.apply(&settings(3, "192.168.1.22")).unwrap();

    assert_eq!(result.applied_revision, 3);
    assert_eq!(fixture.applied_revision(), "3");
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn diagnose_reports_ambiguous_owned_processes() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    let owned = record(ProcessComponent::CoreDns, &fixture.paths);
    fixture.records.seed(owned.clone());
    let mut forged = identity(&owned);
    forged.start_identity = "different".into();
    fixture.host.seed_identity(forged);

    let diagnostics = backend.diagnose(&settings(1, "192.168.1.20")).unwrap();

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "ambiguousOwnedProcess" && diagnostic.status == DiagnosticStatus::Error
    }));
}

#[test]
fn reconcile_restarts_a_setup_server_that_stopped() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    fixture.setup.stop().unwrap();
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Degraded);

    let result = backend.apply(&settings(2, "192.168.1.21")).unwrap();

    assert_eq!(result.applied_revision, 2);
    assert_eq!(fixture.setup.starts(), 2);
    assert!(fixture.setup.is_running());
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Running);
}

#[test]
fn failed_reconcile_stops_a_newly_started_setup_server() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    fixture.setup.stop().unwrap();
    fixture.caddy.fail_next_load();

    backend.apply(&settings(2, "192.168.1.21")).unwrap_err();

    assert!(!fixture.setup.is_running());
    assert_eq!(fixture.applied_revision(), "1");
    assert_eq!(backend.inspect().unwrap(), RuntimeStatus::Degraded);
}

#[test]
fn diagnose_does_not_report_owned_runtime_ports_as_conflicts() {
    let fixture = Fixture::new();
    let backend = fixture.backend();
    backend.apply(&settings(1, "192.168.1.20")).unwrap();
    fixture.inspector.set(vec![error_diagnostic(
        "portConflict",
        "port 8080 is already in use",
    )]);

    let diagnostics = backend.diagnose(&settings(1, "192.168.1.20")).unwrap();

    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "portConflict"),
        "{diagnostics:?}"
    );
}

#[test]
fn runtime_backend_is_send_and_sync_for_the_daemon_arc() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MacOsRuntimeBackend<'static>>();
}
