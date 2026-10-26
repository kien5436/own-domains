use own_domains_daemon::platform::macos::{
    CaddyAdminClient, ChildProcess, DnsReadinessProbe, HttpReadinessProbe, ListenerBinding,
    MacOsArchitecture, OwnedProcessRecord, ProcessComponent, ProcessError, ProcessHost,
    ProcessIdentity, ProcessRecords, ProcessState, ProcessSupervisor, ReadinessProbe, RetryPolicy,
    RuntimePaths, SpawnSpec, Waiter,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, UdpSocket},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

#[derive(Clone)]
struct ChildControl {
    pid: u32,
    signals: Arc<Mutex<Vec<i32>>>,
    exited: Arc<Mutex<bool>>,
    exit_on_term: bool,
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
        if (signal == SIGTERM && self.control.exit_on_term) || signal == SIGKILL {
            *self.control.exited.lock().unwrap() = true;
        }
        Ok(())
    }

    fn try_wait(&mut self) -> Result<Option<i32>, String> {
        Ok(self.control.exited.lock().unwrap().then_some(0))
    }

    fn wait(&mut self) -> Result<i32, String> {
        while !*self.control.exited.lock().unwrap() {
            std::thread::yield_now();
        }
        Ok(0)
    }
}

#[derive(Default)]
struct FakeHost {
    state: Mutex<HostState>,
}

#[derive(Default)]
struct HostState {
    spawns: Vec<SpawnSpec>,
    children: Vec<ChildControl>,
    identities: BTreeMap<u32, Option<ProcessIdentity>>,
    signals: Vec<(u32, i32)>,
    listeners: Vec<ListenerBinding>,
    spawn_error: Option<String>,
    missing_identity_after_spawn: bool,
    exit_on_term: bool,
    remove_identity_on_term: bool,
    remove_identity_on_kill: bool,
}

impl FakeHost {
    fn seed_identity(&self, identity: ProcessIdentity) {
        self.state
            .lock()
            .unwrap()
            .identities
            .insert(identity.pid, Some(identity));
    }

    fn clear_identity(&self, pid: u32) {
        self.state.lock().unwrap().identities.remove(&pid);
    }

    fn set_listeners(&self, listeners: Vec<ListenerBinding>) {
        self.state.lock().unwrap().listeners = listeners;
    }

    fn missing_identity_after_spawn(&self) {
        self.state.lock().unwrap().missing_identity_after_spawn = true;
    }

    fn exit_on_term(&self, exit: bool) {
        self.state.lock().unwrap().exit_on_term = exit;
    }

    fn remove_identity_on_term(&self) {
        self.state.lock().unwrap().remove_identity_on_term = true;
    }

    fn remove_identity_on_kill(&self) {
        self.state.lock().unwrap().remove_identity_on_kill = true;
    }

    fn spawns(&self) -> Vec<SpawnSpec> {
        self.state.lock().unwrap().spawns.clone()
    }

    fn signals(&self) -> Vec<(u32, i32)> {
        self.state.lock().unwrap().signals.clone()
    }

    fn child_signals(&self, index: usize) -> Vec<i32> {
        self.state.lock().unwrap().children[index]
            .signals
            .lock()
            .unwrap()
            .clone()
    }
}

impl ProcessHost for FakeHost {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn ChildProcess>, String> {
        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.spawn_error.take() {
            return Err(message);
        }
        let pid = 1000 + state.spawns.len() as u32;
        let process_group = pid;
        state.spawns.push(spec.clone());
        let identity = ProcessIdentity {
            pid,
            process_group,
            executable: spec.program.clone(),
            arguments: spec.arguments.clone(),
            start_identity: format!("start-{pid}"),
        };
        let missing_identity = state.missing_identity_after_spawn;
        state
            .identities
            .insert(pid, (!missing_identity).then_some(identity));
        let control = ChildControl {
            pid,
            signals: Arc::new(Mutex::new(Vec::new())),
            exited: Arc::new(Mutex::new(false)),
            exit_on_term: state.exit_on_term,
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
        if signal == SIGKILL && state.remove_identity_on_kill {
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
struct FakeWaiter {
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeWaiter {
    fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
    }
}

impl Waiter for FakeWaiter {
    fn sleep(&self, duration: Duration) {
        self.sleeps.lock().unwrap().push(duration);
    }
}

struct FakeProbe {
    component: &'static str,
    results: Mutex<Vec<Result<(), String>>>,
    calls: Mutex<usize>,
}

impl FakeProbe {
    fn new(component: &'static str, results: Vec<Result<(), String>>) -> Self {
        Self {
            component,
            results: Mutex::new(results),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl ReadinessProbe for FakeProbe {
    fn component(&self) -> &'static str {
        self.component
    }

    fn check(&self) -> Result<(), String> {
        *self.calls.lock().unwrap() += 1;
        let mut results = self.results.lock().unwrap();
        if results.is_empty() {
            Err("no more results".into())
        } else {
            results.remove(0)
        }
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
        arguments: match component {
            ProcessComponent::CoreDns => vec![
                "-conf".into(),
                paths
                    .applied_directory
                    .join("Corefile")
                    .to_string_lossy()
                    .into_owned(),
            ],
            ProcessComponent::Caddy => vec![
                "run".into(),
                "--config".into(),
                paths
                    .applied_directory
                    .join("caddy.json")
                    .to_string_lossy()
                    .into_owned(),
                "--adapter".into(),
                "json".into(),
            ],
        },
        start_identity: "start-42".into(),
        log_path: paths.log_directory.join(match component {
            ProcessComponent::CoreDns => "coredns.log",
            ProcessComponent::Caddy => "caddy.log",
        }),
    }
}

fn matching_identity(record: &OwnedProcessRecord) -> ProcessIdentity {
    ProcessIdentity {
        pid: record.pid,
        process_group: record.process_group,
        executable: record.executable.clone(),
        arguments: record.arguments.clone(),
        start_identity: record.start_identity.clone(),
    }
}

fn make_supervisor<'a>(
    paths: &'a RuntimePaths,
    host: &'a FakeHost,
    records: &'a FakeRecords,
    waiter: &'a FakeWaiter,
) -> ProcessSupervisor<'a> {
    let mut supervisor = ProcessSupervisor::new(paths.clone(), host, records, waiter);
    supervisor.readiness = RetryPolicy {
        attempts: 3,
        interval: Duration::from_millis(5),
    };
    supervisor.graceful_stop = RetryPolicy {
        attempts: 2,
        interval: Duration::from_millis(5),
    };
    supervisor.forced_stop = RetryPolicy {
        attempts: 2,
        interval: Duration::from_millis(5),
    };
    supervisor
}

#[test]
fn starts_coredns_with_minimal_environment_fixed_arguments_and_durable_record() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let corefile = paths.applied_directory.join("Corefile");

    let started = supervisor
        .start(ProcessComponent::CoreDns, &corefile)
        .unwrap();

    assert_eq!(started.pid, 1000);
    assert_eq!(started.process_group, 1000);
    assert_eq!(started.executable, paths.coredns_binary);
    assert_eq!(
        started.arguments,
        vec![
            "-conf".to_string(),
            corefile.to_string_lossy().into_owned(),
            "-quiet".to_string(),
        ]
    );
    assert_eq!(started.log_path, paths.log_directory.join("coredns.log"));
    assert_eq!(started.start_identity, "start-1000");
    assert_eq!(
        records.read(ProcessComponent::CoreDns).unwrap(),
        Some(started)
    );
    assert!(matches!(
        supervisor
            .inspect_component(ProcessComponent::CoreDns)
            .unwrap(),
        ProcessState::Running(_)
    ));

    let spawn = &host.spawns()[0];
    assert_eq!(spawn.program, paths.coredns_binary);
    assert_eq!(spawn.working_directory, paths.runtime_directory);
    let keys = spawn
        .environment
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["HOME", "TMPDIR"]);
    assert!(host.child_signals(0).is_empty());
}

#[test]
fn starts_caddy_with_native_json_and_owner_only_data_directory() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let config = paths.applied_directory.join("caddy.json");

    let started = supervisor.start(ProcessComponent::Caddy, &config).unwrap();

    assert_eq!(
        started.arguments,
        vec![
            "run".to_string(),
            "--config".to_string(),
            config.to_string_lossy().into_owned(),
        ]
    );
    let spawn = &host.spawns()[0];
    assert_eq!(
        spawn.environment,
        vec![
            (
                "HOME".to_string(),
                std::env::var("HOME").unwrap_or_default()
            ),
            (
                "TMPDIR".to_string(),
                std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into())
            ),
            (
                "XDG_DATA_HOME".to_string(),
                paths.caddy_data_directory.to_string_lossy().into_owned()
            ),
            (
                "XDG_CONFIG_HOME".to_string(),
                paths
                    .state_directory
                    .join("caddy-config")
                    .to_string_lossy()
                    .into_owned()
            ),
        ]
    );
}

#[test]
fn refuses_to_launch_when_spawned_identity_cannot_be_verified() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    host.missing_identity_after_spawn();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);

    let error = supervisor
        .start(
            ProcessComponent::CoreDns,
            &paths.applied_directory.join("Corefile"),
        )
        .unwrap_err();

    assert!(matches!(error, ProcessError::Identity { .. }));
    assert_eq!(host.child_signals(0), vec![SIGKILL]);
    assert_eq!(records.read(ProcessComponent::CoreDns).unwrap(), None);
}

#[test]
fn inspect_reports_stopped_running_stale_and_ambiguous() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let owned = record(ProcessComponent::CoreDns, &paths);

    assert!(matches!(
        supervisor
            .inspect_component(ProcessComponent::CoreDns)
            .unwrap(),
        ProcessState::Stopped
    ));

    records.seed(owned.clone());
    host.seed_identity(matching_identity(&owned));
    assert!(matches!(
        supervisor.inspect_component(ProcessComponent::CoreDns).unwrap(),
        ProcessState::Running(ref record) if record.pid == 42
    ));

    let mut forged = matching_identity(&owned);
    forged.arguments.push("--foreign".into());
    host.seed_identity(forged);
    assert!(matches!(
        supervisor
            .inspect_component(ProcessComponent::CoreDns)
            .unwrap(),
        ProcessState::Ambiguous { .. }
    ));

    host.clear_identity(42);
    assert!(matches!(
        supervisor
            .inspect_component(ProcessComponent::CoreDns)
            .unwrap(),
        ProcessState::Stale { .. }
    ));
}

#[test]
fn stops_a_verified_recovered_child_gracefully_and_removes_its_record() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let owned = record(ProcessComponent::CoreDns, &paths);
    records.seed(owned.clone());
    host.seed_identity(matching_identity(&owned));
    host.remove_identity_on_term();

    supervisor
        .stop_component(ProcessComponent::CoreDns)
        .unwrap();

    assert_eq!(host.signals(), vec![(42, SIGTERM)]);
    assert_eq!(records.read(ProcessComponent::CoreDns).unwrap(), None);
}

#[test]
fn escalates_to_sigkill_for_a_recovered_child_that_ignores_sigterm() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let owned = record(ProcessComponent::Caddy, &paths);
    records.seed(owned.clone());
    host.seed_identity(matching_identity(&owned));
    host.remove_identity_on_kill();

    supervisor.stop_component(ProcessComponent::Caddy).unwrap();

    assert_eq!(host.signals(), vec![(42, SIGTERM), (42, SIGKILL)]);
    assert_eq!(records.read(ProcessComponent::Caddy).unwrap(), None);
}

#[test]
fn never_signals_a_process_whose_identity_does_not_match_the_record() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let owned = record(ProcessComponent::CoreDns, &paths);
    records.seed(owned.clone());
    let mut forged = matching_identity(&owned);
    forged.executable = PathBuf::from("/usr/bin/forged");
    host.seed_identity(forged);

    let error = supervisor
        .stop_component(ProcessComponent::CoreDns)
        .unwrap_err();

    assert!(matches!(error, ProcessError::Ambiguous { .. }));
    assert!(host.signals().is_empty());
    assert!(records.read(ProcessComponent::CoreDns).unwrap().is_some());
}

#[test]
fn stops_a_live_child_gracefully_then_escalates_when_required() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    host.exit_on_term(true);
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    supervisor
        .start(
            ProcessComponent::CoreDns,
            &paths.applied_directory.join("Corefile"),
        )
        .unwrap();

    supervisor
        .stop_component(ProcessComponent::CoreDns)
        .unwrap();

    assert_eq!(host.child_signals(0), vec![SIGTERM]);
    assert_eq!(records.read(ProcessComponent::CoreDns).unwrap(), None);

    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    supervisor
        .start(
            ProcessComponent::Caddy,
            &paths.applied_directory.join("caddy.json"),
        )
        .unwrap();

    supervisor.stop_component(ProcessComponent::Caddy).unwrap();

    assert_eq!(host.child_signals(0), vec![SIGTERM, SIGKILL]);
    assert_eq!(records.read(ProcessComponent::Caddy).unwrap(), None);
}

#[test]
fn reports_owned_listener_survivors_and_keeps_the_record_for_retry() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let owned = record(ProcessComponent::CoreDns, &paths);
    records.seed(owned.clone());
    host.set_listeners(vec![ListenerBinding {
        port: 5354,
        pid: owned.pid,
        process_group: Some(owned.process_group),
    }]);

    let error = supervisor
        .stop_component(ProcessComponent::CoreDns)
        .unwrap_err();

    assert!(matches!(&error, ProcessError::Stop { message, .. } if message.contains("5354")));
    assert!(host.signals().is_empty());
    assert!(records.read(ProcessComponent::CoreDns).unwrap().is_some());
}

#[test]
fn ignores_listeners_owned_by_another_process_when_stopping() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let owned = record(ProcessComponent::CoreDns, &paths);
    records.seed(owned);
    host.set_listeners(vec![ListenerBinding {
        port: 5354,
        pid: 777,
        process_group: Some(777),
    }]);

    supervisor
        .stop_component(ProcessComponent::CoreDns)
        .unwrap();

    assert_eq!(records.read(ProcessComponent::CoreDns).unwrap(), None);
}

#[test]
fn waits_for_readiness_within_a_bounded_number_of_attempts() {
    let paths = paths();
    let host = FakeHost::default();
    let records = FakeRecords::default();
    let waiter = FakeWaiter::default();
    let supervisor = make_supervisor(&paths, &host, &records, &waiter);
    let probe = FakeProbe::new(
        "CoreDNS",
        vec![
            Err("not listening".into()),
            Err("not listening".into()),
            Ok(()),
        ],
    );

    supervisor.wait_for_readiness(&probe).unwrap();

    assert_eq!(probe.calls(), 3);
    assert_eq!(waiter.sleeps().len(), 2);

    let probe = FakeProbe::new(
        "Caddy",
        vec![
            Err("admin socket missing".into()),
            Err("admin socket missing".into()),
            Err("admin socket missing".into()),
        ],
    );
    let error = supervisor.wait_for_readiness(&probe).unwrap_err();
    assert!(
        matches!(&error, ProcessError::Readiness { component, message } if *component == "Caddy" && message.contains("admin socket"))
    );
    assert_eq!(probe.calls(), 3);
}

#[cfg(unix)]
#[test]
fn loads_caddy_configuration_over_a_restricted_unix_socket() {
    use std::os::unix::net::UnixListener;

    let directory = std::env::temp_dir().join(format!("odc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("admin.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let headers = String::from_utf8_lossy(&request).into_owned();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap();
        let body_start = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        while request.len() < body_start + content_length {
            let read = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..read]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });

    wait_for_socket(&socket);
    let client = CaddyAdminClient::new(socket.clone()).unwrap();
    client
        .load(&json!({"apps": {"http": {"http_port": 8080}}}))
        .unwrap();

    let request = server.join().unwrap();
    assert!(request.starts_with("POST /load HTTP/1.1\r\n"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    assert!(request.contains("\"http_port\":8080"));

    std::fs::remove_file(&socket).unwrap();
    let rejected_socket = socket.clone();
    let rejected = thread::spawn(move || {
        let listener = UnixListener::bind(&rejected_socket).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 13\r\n\r\nbad config!!\n")
            .unwrap();
    });
    wait_for_socket(&socket);
    let error = client.load(&json!({})).unwrap_err();
    assert!(error.contains("400"), "unexpected error: {error}");
    rejected.join().unwrap();

    std::fs::remove_file(&socket).unwrap();
    let oversized_socket = socket.clone();
    let oversized = thread::spawn(move || {
        let listener = UnixListener::bind(&oversized_socket).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n")
            .unwrap();
    });
    wait_for_socket(&socket);
    assert!(client.load(&json!({})).is_err());
    oversized.join().unwrap();

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn dns_probe_requires_both_udp_and_tcp_answers() {
    let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = tcp.local_addr().unwrap().port();
    let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    let expected = Ipv4Addr::new(192, 168, 1, 20);

    let udp_server = thread::spawn(move || {
        let mut buffer = [0_u8; 512];
        let (length, peer) = udp.recv_from(&mut buffer).unwrap();
        udp.send_to(&dns_response(&buffer[..length], expected), peer)
            .unwrap();
    });
    let tcp_server = thread::spawn(move || {
        let (mut stream, _) = tcp.accept().unwrap();
        let mut length = [0_u8; 2];
        stream.read_exact(&mut length).unwrap();
        let mut query = vec![0_u8; u16::from_be_bytes(length) as usize];
        stream.read_exact(&mut query).unwrap();
        let response = dns_response(&query, expected);
        stream
            .write_all(&(response.len() as u16).to_be_bytes())
            .unwrap();
        stream.write_all(&response).unwrap();
    });

    let probe = DnsReadinessProbe::new(Ipv4Addr::LOCALHOST, port, expected, Duration::from_secs(1));
    probe.check().unwrap();

    udp_server.join().unwrap();
    tcp_server.join().unwrap();

    let probe = DnsReadinessProbe::new(
        Ipv4Addr::LOCALHOST,
        port,
        Ipv4Addr::new(10, 0, 0, 1),
        Duration::from_millis(100),
    );
    assert!(probe.check().is_err());
}

#[test]
fn http_probe_checks_status_and_host_header() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let request = String::from_utf8_lossy(&request).into_owned();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .unwrap();
        request
    });

    let probe = HttpReadinessProbe::new(
        Ipv4Addr::LOCALHOST,
        port,
        "setup.own",
        "/fingerprint",
        200,
        Duration::from_secs(1),
    );
    probe.check().unwrap();

    let request = server.join().unwrap();
    assert!(request.starts_with("GET /fingerprint HTTP/1.1\r\n"));
    assert!(request.contains("Host: setup.own\r\n"));
}

fn wait_for_socket(socket: &Path) {
    for _ in 0..100 {
        if socket.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("socket {} was not created", socket.display());
}

fn dns_response(query: &[u8], address: Ipv4Addr) -> Vec<u8> {
    assert!(query.len() >= 12);
    let mut response = query.to_vec();
    response[2] = 0x81;
    response[3] = 0x80;
    response[6] = 0;
    response[7] = 1;
    response.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    response.extend_from_slice(&60_u32.to_be_bytes());
    response.extend_from_slice(&[0x00, 0x04]);
    response.extend_from_slice(&address.octets());
    response
}
