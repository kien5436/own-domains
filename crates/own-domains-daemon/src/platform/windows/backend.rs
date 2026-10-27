//! Windows runtime backend skeleton.
//!
//! Pure orchestration over injected adapters (mirroring the macOS
//! `RuntimeAdapters` seam), so unit tests run everywhere without real
//! sidecars. Process supervision beyond plain spawn/terminate and the host
//! inspector land in later milestones.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use crate::RuntimeError;
use own_domains_core::{ApplyResult, ApplyStatus, Settings, build_corefile, build_proxy_config};
use own_domains_protocol::RuntimeStatus;

use super::config::{windows_dns_options, windows_proxy_options};
#[cfg(windows)]
use super::process::WindowsChild;
use crate::probe::{
    CaddyAdmin, CaddyAdminClient, DnsReadinessProbe, HttpReadinessProbe, ReadinessProbe,
};

pub struct WindowsPaths {
    pub runtime_directory: PathBuf,
    pub coredns_exe: PathBuf,
    pub caddy_exe: PathBuf,
    pub admin_socket: PathBuf,
}

pub trait BackendProcessHost: Send + Sync {
    fn spawn(&self, exe: &Path, args: &[String]) -> Result<Box<dyn BackendChild>, String>;
}

pub trait BackendChild: Send {
    fn terminate(&self) -> Result<(), String>;
    fn wait_exit(&self, timeout: Duration) -> Result<u32, String>;
}

pub trait BackendReadiness: Send + Sync {
    fn wait_dns(&self, address: &str, port: u16, expected: &str) -> Result<(), String>;
    fn wait_http(&self, address: &str, port: u16, host: &str) -> Result<(), String>;
}

pub trait BackendAdmin: Send + Sync {
    fn load(&self, config: &serde_json::Value) -> Result<(), String>;
}

pub struct WindowsAdapters<'a> {
    pub process_host: &'a dyn BackendProcessHost,
    pub readiness: &'a dyn BackendReadiness,
    pub caddy: &'a dyn BackendAdmin,
}

struct BackendState {
    running: bool,
}

pub struct WindowsRuntimeBackend<'a> {
    paths: WindowsPaths,
    adapters: WindowsAdapters<'a>,
    state: Mutex<BackendState>,
    children: Mutex<Vec<Box<dyn BackendChild>>>,
}

impl<'a> WindowsRuntimeBackend<'a> {
    pub fn with_adapters(paths: WindowsPaths, adapters: WindowsAdapters<'a>) -> Self {
        Self {
            paths,
            adapters,
            state: Mutex::new(BackendState { running: false }),
            children: Mutex::new(Vec::new()),
        }
    }

    fn write_text(&self, name: &str, contents: &str) -> Result<PathBuf, RuntimeError> {
        std::fs::create_dir_all(&self.paths.runtime_directory).map_err(failure)?;
        let path = self.paths.runtime_directory.join(name);
        std::fs::write(&path, contents).map_err(failure)?;
        Ok(path)
    }
}

fn failure(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Failure(error.to_string())
}

impl crate::RuntimeBackend for WindowsRuntimeBackend<'_> {
    fn apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        let dns = windows_dns_options(settings).map_err(failure)?;
        let corefile = build_corefile(settings, &dns).map_err(failure)?;
        let corefile_path = self.write_text("Corefile", &corefile)?;
        let proxy = windows_proxy_options(settings, &self.paths.admin_socket).map_err(failure)?;
        let config = build_proxy_config(settings, &proxy).map_err(failure)?;
        let config_text =
            serde_json::to_string_pretty(&config).map_err(|error| failure(error.to_string()))?;
        let config_path = self.write_text("caddy.json", &config_text)?;

        let mut children = self
            .children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        children.push(
            self.adapters
                .process_host
                .spawn(
                    &self.paths.coredns_exe,
                    &[
                        "-conf".to_string(),
                        corefile_path.to_string_lossy().into_owned(),
                    ],
                )
                .map_err(RuntimeError::Failure)?,
        );
        children.push(
            self.adapters
                .process_host
                .spawn(
                    &self.paths.caddy_exe,
                    &[
                        "run".to_string(),
                        "--config".to_string(),
                        config_path.to_string_lossy().into_owned(),
                    ],
                )
                .map_err(RuntimeError::Failure)?,
        );
        drop(children);

        self.adapters
            .readiness
            .wait_dns("127.0.0.1", 53, &settings.lan_address)
            .map_err(RuntimeError::Failure)?;
        self.adapters
            .readiness
            .wait_http("127.0.0.1", 80, "setup.own")
            .map_err(RuntimeError::Failure)?;
        self.adapters
            .caddy
            .load(&config)
            .map_err(RuntimeError::Failure)?;

        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .running = true;
        Ok(ApplyResult {
            desired_revision: settings.revision,
            applied_revision: settings.revision,
            status: ApplyStatus::Applied,
            message: None,
        })
    }

    fn stop(&self) -> Result<(), RuntimeError> {
        let mut children = self
            .children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for child in children.drain(..) {
            child.terminate().map_err(RuntimeError::Failure)?;
            let _ = child.wait_exit(Duration::from_secs(5));
        }
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .running = false;
        Ok(())
    }

    fn inspect(&self) -> Result<RuntimeStatus, RuntimeError> {
        Ok(
            if self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .running
            {
                RuntimeStatus::Running
            } else {
                RuntimeStatus::Stopped
            },
        )
    }

    fn diagnose(
        &self,
        _settings: &Settings,
    ) -> Result<Vec<own_domains_core::Diagnostic>, RuntimeError> {
        Ok(Vec::new())
    }
}

#[cfg(windows)]
/// Quote one command-line argument per the C runtime rules used by
/// `CreateProcessW` with a null application name: arguments containing
/// whitespace are wrapped in quotes; quotes and control characters are
/// rejected rather than escaped.
fn quote_arg(arg: &str) -> Result<String, String> {
    if arg.bytes().any(|b| b == b'"' || b < 0x20) {
        return Err("unsupported character in command argument".to_string());
    }
    Ok(if arg.bytes().any(|b| b == b' ' || b == b'\t') {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    })
}

#[cfg(windows)]
/// Real process host: `CREATE_NO_WINDOW` launches, `TerminateProcess` stops.
pub struct SystemProcessHost;

#[cfg(windows)]
impl BackendProcessHost for SystemProcessHost {
    fn spawn(&self, exe: &Path, args: &[String]) -> Result<Box<dyn BackendChild>, String> {
        let mut command_line = quote_arg(&exe.to_string_lossy())?;
        for arg in args {
            command_line.push(' ');
            command_line.push_str(&quote_arg(arg)?);
        }
        WindowsChild::spawn_no_window(&command_line)
            .map(|child| Box::new(child) as Box<dyn BackendChild>)
            .map_err(|error| format!("cannot launch sidecar: {error}"))
    }
}

#[cfg(windows)]
impl BackendChild for WindowsChild {
    fn terminate(&self) -> Result<(), String> {
        WindowsChild::terminate(self).map_err(|error| error.to_string())
    }

    fn wait_exit(&self, timeout: Duration) -> Result<u32, String> {
        WindowsChild::wait_timeout(self, timeout).map_err(|error| error.to_string())
    }
}

/// Real readiness probes with VM-tolerant timeouts (emulated sidecars and
/// first-run CA generation are slow).
pub struct SystemReadiness;

const READINESS_DEADLINE: Duration = Duration::from_secs(30);
const READINESS_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const READINESS_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Retry `probe` until the deadline. Windows UDP returns WSAECONNRESET
/// (10054) instead of a timeout when the packet arrives before CoreDNS binds,
/// so every error is transient until the deadline expires.
fn wait_for_probe(
    label: &str,
    mut probe: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + READINESS_DEADLINE;
    loop {
        let error = match probe() {
            Ok(()) => return Ok(()),
            Err(message) => message,
        };
        if std::time::Instant::now() >= deadline {
            return Err(format!("{label} was not ready within 30s: {error}"));
        }
        std::thread::sleep(READINESS_RETRY_DELAY);
    }
}

impl BackendReadiness for SystemReadiness {
    fn wait_dns(&self, address: &str, port: u16, expected: &str) -> Result<(), String> {
        let address: std::net::Ipv4Addr = address
            .parse()
            .map_err(|_| format!("invalid probe address: {address}"))?;
        let expected: std::net::Ipv4Addr = expected
            .parse()
            .map_err(|_| format!("invalid expected address: {expected}"))?;
        wait_for_probe("CoreDNS", || {
            DnsReadinessProbe::new(address, port, expected, READINESS_ATTEMPT_TIMEOUT).check()
        })
    }

    fn wait_http(&self, address: &str, port: u16, host: &str) -> Result<(), String> {
        let address: std::net::Ipv4Addr = address
            .parse()
            .map_err(|_| format!("invalid probe address: {address}"))?;
        let host = host.to_string();
        wait_for_probe("HTTP", || {
            HttpReadinessProbe::new(
                address,
                port,
                host.clone(),
                "/",
                200,
                READINESS_ATTEMPT_TIMEOUT,
            )
            .check()
        })
    }
}

impl BackendAdmin for CaddyAdminClient {
    fn load(&self, config: &serde_json::Value) -> Result<(), String> {
        <Self as CaddyAdmin>::load(self, config)
    }
}
