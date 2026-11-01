//! Process supervision shared by Unix adapters.
//!
//! Components, records, policies, spawning, signals, and identity/listener
//! probing. Platform differences live in the cfg-gated `system_*` fns.

use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const SIGTERM: i32 = 15;
pub const SIGKILL: i32 = 9;

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

    pub fn log_name(self) -> &'static str {
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
    pub fn delay_before_next(&self, attempt: u32) -> Option<Duration> {
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

pub fn canonical_program(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_program_resolves_parent_components() {
        let root = std::env::temp_dir().join(format!(
            "own-domains-canonical-program-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let resources = root.join("Contents/Resources");
        let macos = root.join("Contents/MacOS");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::create_dir_all(&macos).unwrap();
        let program = resources.join("coredns-aarch64-apple-darwin");
        std::fs::write(&program, b"").unwrap();
        let unresolved = macos.join("../Resources/coredns-aarch64-apple-darwin");

        assert_eq!(
            canonical_program(&unresolved),
            program.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
