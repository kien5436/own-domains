use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use super::RuntimePaths;

pub const SIGTERM: i32 = 15;
pub const SIGKILL: i32 = 9;

const RECORD_SCHEMA_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessComponent {
    CoreDns,
    Caddy,
}

impl ProcessComponent {
    pub fn name(self) -> &'static str {
        match self {
            Self::CoreDns => "CoreDNS",
            Self::Caddy => "Caddy",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::CoreDns => "coredns",
            Self::Caddy => "caddy",
        }
    }

    pub fn ports(self) -> &'static [u16] {
        match self {
            Self::CoreDns => &[5354],
            Self::Caddy => &[8080, 8443],
        }
    }

    fn log_name(self) -> &'static str {
        match self {
            Self::CoreDns => "coredns.log",
            Self::Caddy => "caddy.log",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedProcessRecord {
    pub schema_version: u32,
    pub component: ProcessComponent,
    pub pid: u32,
    pub process_group: u32,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub start_identity: String,
    pub log_path: PathBuf,
}

impl OwnedProcessRecord {
    pub fn matches(&self, identity: &ProcessIdentity) -> bool {
        self.pid == identity.pid
            && self.process_group == identity.process_group
            && self.executable == identity.executable
            && self.arguments == identity.arguments
            && self.start_identity == identity.start_identity
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub process_group: u32,
    pub executable: PathBuf,
    /// Arguments excluding `argv[0]`.
    pub arguments: Vec<String>,
    pub start_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListenerBinding {
    pub port: u16,
    pub pid: u32,
    pub process_group: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpawnSpec {
    pub component: ProcessComponent,
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: Vec<(String, String)>,
    pub log_path: PathBuf,
}

pub trait ChildProcess: Send {
    fn pid(&self) -> u32;
    fn signal(&mut self, signal: i32) -> Result<(), String>;
    fn try_wait(&mut self) -> Result<Option<i32>, String>;
    fn wait(&mut self) -> Result<i32, String>;
}

pub trait ProcessHost: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn ChildProcess>, String>;
    fn identity(&self, pid: u32) -> Result<Option<ProcessIdentity>, String>;
    fn signal_group(&self, process_group: u32, signal: i32) -> Result<(), String>;
    fn listeners(&self, ports: &[u16]) -> Result<Vec<ListenerBinding>, String>;
}

pub trait ProcessRecords: Send + Sync {
    fn read(&self, component: ProcessComponent) -> Result<Option<OwnedProcessRecord>, String>;
    fn write(&self, record: &OwnedProcessRecord) -> Result<(), String>;
    fn remove(&self, component: ProcessComponent) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub interval: Duration,
}

impl RetryPolicy {
    fn delay_before_next(&self, attempt: u32) -> Option<Duration> {
        (attempt + 1 < self.attempts).then_some(self.interval)
    }
}

pub trait Waiter: Send + Sync {
    fn sleep(&self, duration: Duration);
}

pub struct SystemWaiter;

impl Waiter for SystemWaiter {
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Stopped,
    Running(OwnedProcessRecord),
    Stale {
        record: OwnedProcessRecord,
        reason: String,
    },
    Ambiguous {
        record: OwnedProcessRecord,
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("{component} failed to start: {message}")]
    Spawn {
        component: &'static str,
        message: String,
    },
    #[error("{component} identity check failed: {message}")]
    Identity {
        component: &'static str,
        message: String,
    },
    #[error("{component} is already recorded with pid {pid}")]
    AlreadyRunning { component: &'static str, pid: u32 },
    #[error("{component} has an ambiguous survivor with pid {pid}: {reason}")]
    Ambiguous {
        component: &'static str,
        pid: u32,
        reason: String,
    },
    #[error("{component} readiness failed: {message}")]
    Readiness {
        component: &'static str,
        message: String,
    },
    #[error("{component} did not stop completely: {message}")]
    Stop {
        component: &'static str,
        message: String,
    },
    #[error("process record operation failed: {0}")]
    Record(String),
}

pub struct ProcessSupervisor<'a> {
    paths: RuntimePaths,
    host: &'a dyn ProcessHost,
    records: &'a dyn ProcessRecords,
    waiter: &'a dyn Waiter,
    children: Mutex<HashMap<ProcessComponent, Box<dyn ChildProcess>>>,
    pub readiness: RetryPolicy,
    pub graceful_stop: RetryPolicy,
    pub forced_stop: RetryPolicy,
}

impl<'a> ProcessSupervisor<'a> {
    pub fn new(
        paths: RuntimePaths,
        host: &'a dyn ProcessHost,
        records: &'a dyn ProcessRecords,
        waiter: &'a dyn Waiter,
    ) -> Self {
        Self {
            paths,
            host,
            records,
            waiter,
            children: Mutex::new(HashMap::new()),
            readiness: RetryPolicy {
                attempts: 50,
                interval: Duration::from_millis(100),
            },
            graceful_stop: RetryPolicy {
                attempts: 30,
                interval: Duration::from_millis(100),
            },
            forced_stop: RetryPolicy {
                attempts: 20,
                interval: Duration::from_millis(100),
            },
        }
    }

    pub fn start(
        &self,
        component: ProcessComponent,
        configuration: &Path,
    ) -> Result<OwnedProcessRecord, ProcessError> {
        if let Some(record) = self.records.read(component).map_err(ProcessError::Record)? {
            return Err(ProcessError::AlreadyRunning {
                component: component.name(),
                pid: record.pid,
            });
        }

        let spec = self.spawn_spec(component, configuration);
        let mut child = self
            .host
            .spawn(&spec)
            .map_err(|message| ProcessError::Spawn {
                component: component.name(),
                message,
            })?;
        let pid = child.pid();
        let identity = match self.host.identity(pid) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                let _ = child.signal(SIGKILL);
                let _ = child.wait();
                return Err(ProcessError::Identity {
                    component: component.name(),
                    message: "the spawned process disappeared before it could be verified".into(),
                });
            }
            Err(message) => {
                let _ = child.signal(SIGKILL);
                let _ = child.wait();
                return Err(ProcessError::Identity {
                    component: component.name(),
                    message,
                });
            }
        };
        if identity.executable != spec.program || identity.arguments != spec.arguments {
            let _ = child.signal(SIGKILL);
            let _ = child.wait();
            return Err(ProcessError::Identity {
                component: component.name(),
                message: "the spawned process does not match the requested executable or arguments"
                    .into(),
            });
        }

        let record = OwnedProcessRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            component,
            pid,
            process_group: identity.process_group,
            executable: identity.executable,
            arguments: identity.arguments,
            start_identity: identity.start_identity,
            log_path: spec.log_path,
        };
        if let Err(message) = self.records.write(&record) {
            let _ = child.signal(SIGKILL);
            let _ = child.wait();
            return Err(ProcessError::Record(message));
        }
        self.children.lock().unwrap().insert(component, child);
        Ok(record)
    }

    pub fn inspect_component(
        &self,
        component: ProcessComponent,
    ) -> Result<ProcessState, ProcessError> {
        let Some(record) = self.records.read(component).map_err(ProcessError::Record)? else {
            return Ok(ProcessState::Stopped);
        };
        match self.host.identity(record.pid) {
            Ok(None) => Ok(ProcessState::Stale {
                record,
                reason: "the recorded process is no longer running".into(),
            }),
            Ok(Some(identity)) if record.matches(&identity) => Ok(ProcessState::Running(record)),
            Ok(Some(_)) => Ok(ProcessState::Ambiguous {
                record,
                reason: "the process identity does not match the ownership record".into(),
            }),
            Err(message) => Ok(ProcessState::Ambiguous {
                record,
                reason: format!("the process identity could not be inspected: {message}"),
            }),
        }
    }

    pub fn stop_component(&self, component: ProcessComponent) -> Result<(), ProcessError> {
        let Some(record) = self.records.read(component).map_err(ProcessError::Record)? else {
            return Ok(());
        };

        let child = self.children.lock().unwrap().remove(&component);
        if let Some(mut child) = child {
            self.stop_live(component, child.as_mut())?;
        } else {
            self.stop_recovered(component, &record)?;
        }

        self.verify_no_owned_listeners(component, &record)?;
        self.records
            .remove(component)
            .map_err(ProcessError::Record)?;
        Ok(())
    }

    pub fn wait_for_readiness(&self, probe: &dyn ReadinessProbe) -> Result<(), ProcessError> {
        let mut last = String::from("the probe did not run");
        for attempt in 0..self.readiness.attempts {
            match probe.check() {
                Ok(()) => return Ok(()),
                Err(message) => last = message,
            }
            if let Some(delay) = self.readiness.delay_before_next(attempt) {
                self.waiter.sleep(delay);
            }
        }
        Err(ProcessError::Readiness {
            component: probe.component(),
            message: last,
        })
    }

    fn stop_live(
        &self,
        component: ProcessComponent,
        child: &mut dyn ChildProcess,
    ) -> Result<(), ProcessError> {
        child
            .signal(SIGTERM)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGTERM failed: {message}"),
            })?;
        if self.wait_for_exit(
            || {
                child
                    .try_wait()
                    .map(|status| status.is_some())
                    .map_err(|message| ProcessError::Stop {
                        component: component.name(),
                        message,
                    })
            },
            self.graceful_stop,
        )? {
            return Ok(());
        }
        child
            .signal(SIGKILL)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGKILL failed: {message}"),
            })?;
        if self.wait_for_exit(
            || {
                child
                    .try_wait()
                    .map(|status| status.is_some())
                    .map_err(|message| ProcessError::Stop {
                        component: component.name(),
                        message,
                    })
            },
            self.forced_stop,
        )? {
            return Ok(());
        }
        Err(ProcessError::Stop {
            component: component.name(),
            message: "the process did not exit after SIGKILL".into(),
        })
    }

    fn stop_recovered(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
    ) -> Result<(), ProcessError> {
        if self.identity_checked(component, record)?.is_none() {
            return Ok(());
        }

        self.host
            .signal_group(record.process_group, SIGTERM)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGTERM failed: {message}"),
            })?;
        if self.wait_for_identity_gone(component, record, self.graceful_stop)? {
            return Ok(());
        }

        // Re-verify immediately before escalation. A reused PID group is not ours.
        if self.identity_checked(component, record)?.is_none() {
            return Ok(());
        }
        self.host
            .signal_group(record.process_group, SIGKILL)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGKILL failed: {message}"),
            })?;
        if self.wait_for_identity_gone(component, record, self.forced_stop)? {
            return Ok(());
        }
        Err(ProcessError::Stop {
            component: component.name(),
            message: "the process identity remained after SIGKILL".into(),
        })
    }

    fn wait_for_identity_gone(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
        policy: RetryPolicy,
    ) -> Result<bool, ProcessError> {
        self.wait_for_exit(
            || Ok(self.identity_checked(component, record)?.is_none()),
            policy,
        )
    }

    fn identity_checked(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
    ) -> Result<Option<ProcessIdentity>, ProcessError> {
        match self.host.identity(record.pid) {
            Ok(None) => Ok(None),
            Ok(Some(identity)) if record.matches(&identity) => Ok(Some(identity)),
            Ok(Some(_)) => Err(ProcessError::Ambiguous {
                component: component.name(),
                pid: record.pid,
                reason: "the process identity changed before a signal was sent".into(),
            }),
            Err(message) => Err(ProcessError::Ambiguous {
                component: component.name(),
                pid: record.pid,
                reason: format!("the process identity could not be inspected: {message}"),
            }),
        }
    }

    fn wait_for_exit(
        &self,
        mut exited: impl FnMut() -> Result<bool, ProcessError>,
        policy: RetryPolicy,
    ) -> Result<bool, ProcessError> {
        for attempt in 0..policy.attempts {
            if exited()? {
                return Ok(true);
            }
            if let Some(delay) = policy.delay_before_next(attempt) {
                self.waiter.sleep(delay);
            }
        }
        Ok(false)
    }

    fn verify_no_owned_listeners(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
    ) -> Result<(), ProcessError> {
        let listeners =
            self.host
                .listeners(component.ports())
                .map_err(|message| ProcessError::Stop {
                    component: component.name(),
                    message: format!("listener inspection failed: {message}"),
                })?;
        let survivors = listeners
            .into_iter()
            .filter(|listener| {
                listener.pid == record.pid || listener.process_group == Some(record.process_group)
            })
            .collect::<Vec<_>>();
        if survivors.is_empty() {
            return Ok(());
        }
        let detail = survivors
            .iter()
            .map(|listener| format!("port {} (pid {})", listener.port, listener.pid))
            .collect::<Vec<_>>()
            .join(", ");
        Err(ProcessError::Stop {
            component: component.name(),
            message: format!("owned listeners remain: {detail}"),
        })
    }

    fn spawn_spec(&self, component: ProcessComponent, configuration: &Path) -> SpawnSpec {
        let configuration = configuration.to_string_lossy().into_owned();
        let (program, arguments) = match component {
            ProcessComponent::CoreDns => (
                self.paths.coredns_binary.clone(),
                vec!["-conf".into(), configuration, "-quiet".into()],
            ),
            ProcessComponent::Caddy => (
                self.paths.caddy_binary.clone(),
                vec!["run".into(), "--config".into(), configuration],
            ),
        };
        let mut environment = vec![
            ("HOME".into(), std::env::var("HOME").unwrap_or_default()),
            (
                "TMPDIR".into(),
                std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()),
            ),
        ];
        if component == ProcessComponent::Caddy {
            environment.push((
                "XDG_DATA_HOME".into(),
                self.paths
                    .caddy_data_directory
                    .to_string_lossy()
                    .into_owned(),
            ));
            environment.push((
                "XDG_CONFIG_HOME".into(),
                self.paths
                    .state_directory
                    .join("caddy-config")
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        SpawnSpec {
            component,
            program,
            arguments,
            working_directory: self.paths.runtime_directory.clone(),
            environment,
            log_path: self.paths.log_directory.join(component.log_name()),
        }
    }
}

pub trait ReadinessProbe: Send + Sync {
    fn component(&self) -> &'static str;
    fn check(&self) -> Result<(), String>;
}

pub struct DnsReadinessProbe {
    address: Ipv4Addr,
    port: u16,
    expected: Ipv4Addr,
    timeout: Duration,
}

impl DnsReadinessProbe {
    pub fn new(address: Ipv4Addr, port: u16, expected: Ipv4Addr, timeout: Duration) -> Self {
        Self {
            address,
            port,
            expected,
            timeout,
        }
    }

    fn query(&self) -> Vec<u8> {
        let mut query = Vec::with_capacity(32);
        query.extend_from_slice(&0x4f44_u16.to_be_bytes());
        query.extend_from_slice(&0x0100_u16.to_be_bytes());
        query.extend_from_slice(&1_u16.to_be_bytes());
        query.extend_from_slice(&0_u16.to_be_bytes());
        query.extend_from_slice(&0_u16.to_be_bytes());
        query.extend_from_slice(&0_u16.to_be_bytes());
        for label in ["setup", "own"] {
            query.push(label.len() as u8);
            query.extend_from_slice(label.as_bytes());
        }
        query.push(0);
        query.extend_from_slice(&1_u16.to_be_bytes());
        query.extend_from_slice(&1_u16.to_be_bytes());
        query
    }
}

impl ReadinessProbe for DnsReadinessProbe {
    fn component(&self) -> &'static str {
        "CoreDNS"
    }

    fn check(&self) -> Result<(), String> {
        let query = self.query();
        self.check_udp(&query)?;
        self.check_tcp(&query)
    }
}

impl DnsReadinessProbe {
    fn check_udp(&self, query: &[u8]) -> Result<(), String> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|error| format!("unable to bind UDP probe socket: {error}"))?;
        socket
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the UDP probe timeout: {error}"))?;
        socket
            .send_to(query, (self.address, self.port))
            .map_err(|error| format!("DNS UDP request failed: {error}"))?;
        let mut response = [0_u8; 4096];
        let (length, _) = socket
            .recv_from(&mut response)
            .map_err(|error| format!("DNS UDP response failed: {error}"))?;
        check_dns_answer(&response[..length], self.expected)
    }

    fn check_tcp(&self, query: &[u8]) -> Result<(), String> {
        let socket = SocketAddr::from((self.address, self.port));
        let mut stream = TcpStream::connect_timeout(&socket, self.timeout)
            .map_err(|error| format!("DNS TCP connection failed: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the DNS TCP timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the DNS TCP timeout: {error}"))?;
        stream
            .write_all(&(query.len() as u16).to_be_bytes())
            .map_err(|error| format!("DNS TCP request failed: {error}"))?;
        stream
            .write_all(query)
            .map_err(|error| format!("DNS TCP request failed: {error}"))?;
        let mut length = [0_u8; 2];
        stream
            .read_exact(&mut length)
            .map_err(|error| format!("DNS TCP response failed: {error}"))?;
        let mut response = vec![0_u8; u16::from_be_bytes(length) as usize];
        stream
            .read_exact(&mut response)
            .map_err(|error| format!("DNS TCP response failed: {error}"))?;
        check_dns_answer(&response, self.expected)
    }
}

pub struct HttpReadinessProbe {
    address: Ipv4Addr,
    port: u16,
    host: String,
    path: String,
    expected_status: u16,
    timeout: Duration,
}

impl HttpReadinessProbe {
    pub fn new(
        address: Ipv4Addr,
        port: u16,
        host: impl Into<String>,
        path: impl Into<String>,
        expected_status: u16,
        timeout: Duration,
    ) -> Self {
        Self {
            address,
            port,
            host: host.into(),
            path: path.into(),
            expected_status,
            timeout,
        }
    }
}

impl ReadinessProbe for HttpReadinessProbe {
    fn component(&self) -> &'static str {
        "HTTP"
    }

    fn check(&self) -> Result<(), String> {
        let socket = SocketAddr::from((self.address, self.port));
        let mut stream = TcpStream::connect_timeout(&socket, self.timeout)
            .map_err(|error| format!("HTTP connection failed: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the HTTP timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the HTTP timeout: {error}"))?;
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.path, self.host
        )
        .map_err(|error| format!("HTTP request failed: {error}"))?;
        let (status, _) = read_http_response(&mut stream, MAX_HTTP_BODY_BYTES)?;
        if status != self.expected_status {
            return Err(format!(
                "HTTP response status {status}, expected {}",
                self.expected_status
            ));
        }
        Ok(())
    }
}

pub trait CaddyAdmin: Send + Sync {
    fn load(&self, config: &Value) -> Result<(), String>;
    fn is_ready(&self) -> Result<(), String>;
}

pub struct CaddyAdminClient {
    socket: PathBuf,
    timeout: Duration,
    max_response_bytes: usize,
}

impl CaddyAdminClient {
    pub fn new(socket: impl Into<PathBuf>) -> Result<Self, String> {
        let socket = socket.into();
        if !socket.is_absolute() {
            return Err("Caddy admin socket must be absolute".into());
        }
        Ok(Self {
            socket,
            timeout: Duration::from_secs(2),
            max_response_bytes: MAX_HTTP_BODY_BYTES,
        })
    }

    pub fn load(&self, config: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(config)
            .map_err(|error| format!("unable to serialize Caddy configuration: {error}"))?;
        self.request("POST", "/load", Some(&body), Some("application/json"))
            .map(|_| ())
    }

    pub fn is_ready(&self) -> Result<(), String> {
        self.request("GET", "/config/", None, None).map(|_| ())
    }

    #[cfg(unix)]
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        use std::os::unix::net::UnixStream;

        let mut stream = UnixStream::connect(&self.socket)
            .map_err(|error| format!("unable to connect to the Caddy admin socket: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the Caddy admin timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the Caddy admin timeout: {error}"))?;
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
        if let Some(content_type) = content_type {
            request.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        if let Some(body) = body {
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("Caddy admin request failed: {error}"))?;
        if let Some(body) = body {
            stream
                .write_all(body)
                .map_err(|error| format!("Caddy admin request failed: {error}"))?;
        }
        let (status, response) = read_http_response(&mut stream, self.max_response_bytes)?;
        if !(200..300).contains(&status) {
            let detail = String::from_utf8_lossy(&response)
                .chars()
                .take(240)
                .collect::<String>();
            return Err(format!("Caddy admin returned status {status}: {detail}"));
        }
        Ok(response)
    }

    #[cfg(not(unix))]
    fn request(
        &self,
        _method: &str,
        _path: &str,
        _body: Option<&[u8]>,
        _content_type: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        Err("Caddy admin Unix sockets are unsupported on this platform".into())
    }
}

impl CaddyAdmin for CaddyAdminClient {
    fn load(&self, config: &Value) -> Result<(), String> {
        CaddyAdminClient::load(self, config)
    }

    fn is_ready(&self) -> Result<(), String> {
        CaddyAdminClient::is_ready(self)
    }
}

pub struct SystemProcessHost;

impl ProcessHost for SystemProcessHost {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn ChildProcess>, String> {
        system_spawn(spec)
    }

    fn identity(&self, pid: u32) -> Result<Option<ProcessIdentity>, String> {
        system_identity(pid)
    }

    fn signal_group(&self, process_group: u32, signal: i32) -> Result<(), String> {
        system_signal_group(process_group, signal)
    }

    fn listeners(&self, ports: &[u16]) -> Result<Vec<ListenerBinding>, String> {
        system_listeners(ports)
    }
}

pub struct FileProcessRecords {
    directory: PathBuf,
}

impl FileProcessRecords {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            directory: paths.runtime_directory.join("processes"),
        }
    }

    fn path(&self, component: ProcessComponent) -> PathBuf {
        self.directory.join(format!("{}.json", component.key()))
    }
}

impl ProcessRecords for FileProcessRecords {
    fn read(&self, component: ProcessComponent) -> Result<Option<OwnedProcessRecord>, String> {
        let path = self.path(component);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("unable to inspect {}: {error}", path.display())),
        };
        if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
            return Err(format!("process record is invalid: {}", path.display()));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
        let record: OwnedProcessRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("unable to parse {}: {error}", path.display()))?;
        if record.schema_version != RECORD_SCHEMA_VERSION || record.component != component {
            return Err(format!(
                "process record is incompatible: {}",
                path.display()
            ));
        }
        Ok(Some(record))
    }

    fn write(&self, record: &OwnedProcessRecord) -> Result<(), String> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|error| format!("unable to create process records: {error}"))?;
        let path = self.path(record.component);
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| format!("unable to serialize process record: {error}"))?;
        write_atomic(&temporary, &path, &bytes, 0o600)
    }

    fn remove(&self, component: ProcessComponent) -> Result<(), String> {
        let path = self.path(component);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("unable to remove {}: {error}", path.display())),
        }
    }
}

fn check_dns_answer(response: &[u8], expected: Ipv4Addr) -> Result<(), String> {
    if response.len() < 12 {
        return Err("DNS response is truncated".into());
    }
    if response[2] & 0x80 == 0 {
        return Err("DNS response is not marked as a response".into());
    }
    let rcode = response[3] & 0x0f;
    if rcode != 0 {
        return Err(format!("DNS response returned error code {rcode}"));
    }
    let questions = u16::from_be_bytes([response[4], response[5]]) as usize;
    let answers = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_dns_name(response, offset)?;
        if offset + 4 > response.len() {
            return Err("DNS response question is truncated".into());
        }
        offset += 4;
    }
    for _ in 0..answers {
        offset = skip_dns_name(response, offset)?;
        if offset + 10 > response.len() {
            return Err("DNS response answer is truncated".into());
        }
        let record_type = u16::from_be_bytes([response[offset], response[offset + 1]]);
        let class = u16::from_be_bytes([response[offset + 2], response[offset + 3]]);
        let length = u16::from_be_bytes([response[offset + 8], response[offset + 9]]) as usize;
        offset += 10;
        if offset + length > response.len() {
            return Err("DNS response answer data is truncated".into());
        }
        if record_type == 1 && class == 1 && length == 4 {
            let address = Ipv4Addr::new(
                response[offset],
                response[offset + 1],
                response[offset + 2],
                response[offset + 3],
            );
            if address == expected {
                return Ok(());
            }
        }
        offset += length;
    }
    Err(format!("DNS response did not contain A {expected}"))
}

fn skip_dns_name(response: &[u8], mut offset: usize) -> Result<usize, String> {
    loop {
        let length = *response
            .get(offset)
            .ok_or_else(|| "DNS response name is truncated".to_string())?;
        if length & 0xc0 == 0xc0 {
            if offset + 1 >= response.len() {
                return Err("DNS response compression pointer is truncated".into());
            }
            return Ok(offset + 2);
        }
        offset += 1;
        if length == 0 {
            return Ok(offset);
        }
        offset += length as usize;
        if offset > response.len() {
            return Err("DNS response name is truncated".into());
        }
    }
}

fn read_http_response(
    stream: &mut impl Read,
    max_body_bytes: usize,
) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("HTTP response read failed: {error}"))?;
        if read == 0 {
            return Err("HTTP response ended before its headers".into());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > MAX_HTTP_HEADER_BYTES {
            return Err("HTTP response headers exceeded 16 KiB".into());
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| "HTTP response status line is invalid".to_string())?;
    let content_length = headers.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .strip_prefix("content-length:")
            .and_then(|value| value.trim().parse::<usize>().ok())
    });
    if content_length.is_some_and(|length| length > max_body_bytes) {
        return Err(format!(
            "HTTP response body exceeded {max_body_bytes} bytes"
        ));
    }

    let mut body = buffer[header_end..].to_vec();
    if body.len() > max_body_bytes {
        return Err(format!(
            "HTTP response body exceeded {max_body_bytes} bytes"
        ));
    }
    if let Some(length) = content_length {
        while body.len() < length {
            let read = stream
                .read(&mut chunk)
                .map_err(|error| format!("HTTP response read failed: {error}"))?;
            if read == 0 {
                return Err("HTTP response body ended early".into());
            }
            body.extend_from_slice(&chunk[..read]);
            if body.len() > max_body_bytes || body.len() > length {
                return Err(format!(
                    "HTTP response body exceeded {max_body_bytes} bytes"
                ));
            }
        }
        body.truncate(length);
    } else {
        loop {
            let read = stream
                .read(&mut chunk)
                .map_err(|error| format!("HTTP response read failed: {error}"))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
            if body.len() > max_body_bytes {
                return Err(format!(
                    "HTTP response body exceeded {max_body_bytes} bytes"
                ));
            }
        }
    }
    Ok((status, body))
}

fn write_atomic(
    temporary: &Path,
    destination: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(temporary)
            .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("unable to flush {}: {error}", temporary.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(temporary, bytes)
            .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
    }
    std::fs::rename(temporary, destination).map_err(|error| {
        let _ = std::fs::remove_file(temporary);
        format!("unable to publish {}: {error}", destination.display())
    })
}

#[cfg(unix)]
fn system_spawn(spec: &SpawnSpec) -> Result<Box<dyn ChildProcess>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    if let Some(parent) = spec.log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("unable to create log directory: {error}"))?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&spec.log_path)
        .map_err(|error| format!("unable to open {}: {error}", spec.log_path.display()))?;
    let errors = log
        .try_clone()
        .map_err(|error| format!("unable to duplicate the log handle: {error}"))?;
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.arguments)
        .current_dir(&spec.working_directory)
        .env_clear()
        .envs(spec.environment.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errors))
        .process_group(0);
    let child = command
        .spawn()
        .map_err(|error| format!("unable to spawn {}: {error}", spec.program.display()))?;
    let pid = child.id();
    Ok(Box::new(SystemChild {
        child,
        pid,
        process_group: pid,
    }))
}

#[cfg(not(unix))]
fn system_spawn(_spec: &SpawnSpec) -> Result<Box<dyn ChildProcess>, String> {
    Err("process supervision is unsupported on this platform".into())
}

#[cfg(unix)]
struct SystemChild {
    child: std::process::Child,
    pid: u32,
    process_group: u32,
}

#[cfg(unix)]
impl ChildProcess for SystemChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn signal(&mut self, signal: i32) -> Result<(), String> {
        system_signal_group(self.process_group, signal)
    }

    fn try_wait(&mut self) -> Result<Option<i32>, String> {
        self.child
            .try_wait()
            .map_err(|error| format!("unable to wait for pid {}: {error}", self.pid))
            .map(|status| status.map(|status| status.code().unwrap_or(-1)))
    }

    fn wait(&mut self) -> Result<i32, String> {
        self.child
            .wait()
            .map(|status| status.code().unwrap_or(-1))
            .map_err(|error| format!("unable to wait for pid {}: {error}", self.pid))
    }
}

#[cfg(unix)]
fn system_signal_group(process_group: u32, signal: i32) -> Result<(), String> {
    if process_group == 0 || process_group > i32::MAX as u32 {
        return Err("invalid process group".into());
    }
    let result = unsafe { libc::kill(-(process_group as i32), signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "unable to signal process group {process_group}: {error}"
        ))
    }
}

#[cfg(not(unix))]
fn system_signal_group(_process_group: u32, _signal: i32) -> Result<(), String> {
    Err("process supervision is unsupported on this platform".into())
}

#[cfg(target_os = "macos")]
fn system_identity(pid: u32) -> Result<Option<ProcessIdentity>, String> {
    use std::ffi::c_void;

    if pid == 0 || pid > i32::MAX as u32 {
        return Ok(None);
    }
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let result = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size as i32,
        )
    };
    if result != size as i32 {
        return Ok(None);
    }

    let mut path = vec![0_u8; 4096];
    let path_length = unsafe {
        libc::proc_pidpath(
            pid as i32,
            path.as_mut_ptr() as *mut c_void,
            path.len() as u32,
        )
    };
    if path_length <= 0 {
        return Ok(None);
    }
    path.truncate(path_length as usize);
    let executable = PathBuf::from(String::from_utf8_lossy(&path).into_owned());

    let arguments = macos_arguments(pid)?.unwrap_or_default();
    Ok(Some(ProcessIdentity {
        pid,
        process_group: info.pbi_pgid,
        executable,
        arguments,
        start_identity: format!("{}.{:06}", info.pbi_start_tvsec, info.pbi_start_tvusec),
    }))
}

#[cfg(target_os = "macos")]
fn macos_arguments(pid: u32) -> Result<Option<Vec<String>>, String> {
    use std::ffi::c_void;

    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let mut size = 0_usize;
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size < 4 {
        return Ok(None);
    }
    let mut buffer = vec![0_u8; size];
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            buffer.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size < 4 {
        return Ok(None);
    }
    buffer.truncate(size);
    let argc = i32::from_ne_bytes(buffer[0..4].try_into().unwrap()).max(0) as usize;
    let mut offset = 4;
    while offset < buffer.len() && buffer[offset] != 0 {
        offset += 1;
    }
    while offset < buffer.len() && buffer[offset] == 0 {
        offset += 1;
    }
    let mut arguments = Vec::with_capacity(argc);
    for _ in 0..argc {
        if offset >= buffer.len() {
            break;
        }
        let start = offset;
        while offset < buffer.len() && buffer[offset] != 0 {
            offset += 1;
        }
        arguments.push(String::from_utf8_lossy(&buffer[start..offset]).into_owned());
        offset += 1;
    }
    if !arguments.is_empty() {
        arguments.remove(0);
    }
    Ok(Some(arguments))
}

#[cfg(target_os = "linux")]
fn system_identity(pid: u32) -> Result<Option<ProcessIdentity>, String> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("unable to inspect process {pid}: {error}")),
    };
    let close = match stat.rfind(')') {
        Some(position) => position,
        None => return Ok(None),
    };
    let fields = stat[close + 1..].split_whitespace().collect::<Vec<_>>();
    if fields.len() < 20 {
        return Ok(None);
    }
    let process_group = fields[2]
        .parse::<u32>()
        .map_err(|error| format!("invalid process group for {pid}: {error}"))?;
    let start_identity = fields[19].to_string();
    let executable = match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("unable to resolve executable for {pid}: {error}")),
    };
    let command_line = match std::fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(command_line) => command_line,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("unable to inspect arguments for {pid}: {error}")),
    };
    let mut arguments = command_line
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect::<Vec<_>>();
    if !arguments.is_empty() {
        arguments.remove(0);
    }
    Ok(Some(ProcessIdentity {
        pid,
        process_group,
        executable,
        arguments,
        start_identity,
    }))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn system_identity(_pid: u32) -> Result<Option<ProcessIdentity>, String> {
    Err("process identity inspection is unsupported on this platform".into())
}

#[cfg(target_os = "macos")]
fn system_listeners(ports: &[u16]) -> Result<Vec<ListenerBinding>, String> {
    let mut listeners = Vec::new();
    for port in ports {
        for (protocol, extra) in [("TCP", vec!["-sTCP:LISTEN"]), ("UDP", Vec::new())] {
            let mut command = std::process::Command::new("/usr/sbin/lsof");
            command
                .args(["-nP", "-t"])
                .arg(format!("-i{protocol}:{port}"));
            command.args(&extra);
            let output = command
                .output()
                .map_err(|error| format!("unable to inspect port {port}: {error}"))?;
            if !output.status.success() {
                continue;
            }
            for pid in String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
            {
                let process_group = system_identity(pid)?.map(|identity| identity.process_group);
                listeners.push(ListenerBinding {
                    port: *port,
                    pid,
                    process_group,
                });
            }
        }
    }
    listeners.sort_by_key(|listener| (listener.port, listener.pid));
    listeners.dedup_by_key(|listener| (listener.port, listener.pid));
    Ok(listeners)
}

#[cfg(not(target_os = "macos"))]
fn system_listeners(_ports: &[u16]) -> Result<Vec<ListenerBinding>, String> {
    Err("listener inspection is only available on macOS".into())
}
