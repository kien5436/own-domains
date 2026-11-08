//! Linux runtime backend.
//!
//! Mirrors `platform::windows::backend`: path resolution plus live
//! orchestration over injected adapters, so unit tests run everywhere
//! without real sidecars.

use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use own_domains_core::{ApplyResult, ApplyStatus, Diagnostic, Settings, build_proxy_config};
use own_domains_protocol::RuntimeStatus;

use super::{
    config::linux_proxy_options,
    manifest::parse_linux_manifest,
    runtime::{setup_manifest_invalid, setup_required},
};
use crate::{
    RuntimeError, SetupServer,
    probe::{CaddyAdmin, CaddyAdminClient, DnsReadinessProbe, HttpReadinessProbe, ReadinessProbe},
};

pub struct LinuxPaths {
    pub runtime_directory: PathBuf,
    pub coredns_exe: PathBuf,
    pub caddy_exe: PathBuf,
    pub caddy_data_directory: PathBuf,
    pub admin_socket: PathBuf,
}

/// Target triple subdirectory under `sidecars/`, matching the fetch output
/// layout (`coredns-<triple>`, no `.exe` on Linux).
pub fn bundled_target() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "unknown-linux-target"
    }
}

/// Installed-bundle resources root for a daemon executable: the exe
/// directory itself when it carries a `sidecars/` bundle layout.
pub fn bundled_resources_dir(exe_dir: &Path) -> Option<PathBuf> {
    if exe_dir.join("sidecars").is_dir() {
        Some(exe_dir.to_path_buf())
    } else {
        None
    }
}

pub fn resolve_paths(
    state_dir: &Path,
    current_dir: &Path,
    exe_dir: &Path,
    resources: Option<&Path>,
    sidecars: Option<PathBuf>,
) -> Result<LinuxPaths, String> {
    let absolutize = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            current_dir.join(path)
        }
    };
    let state_dir = absolutize(state_dir);
    // Each branch resolves full exe file names: plain `coredns`/`caddy`
    // for explicit and exe-adjacent layouts, triple-infixed names for the
    // staged bundle (`coredns-<target>`, mirroring the fetch output).
    let (sidecars, coredns_name, caddy_name) = match sidecars {
        Some(dir) => (absolutize(&dir), "coredns".to_string(), "caddy".to_string()),
        None => match resources {
            Some(resources) => {
                let bundled = resources.join("sidecars").join(bundled_target());
                let names = (
                    format!("coredns-{}", bundled_target()),
                    format!("caddy-{}", bundled_target()),
                );
                if bundled.join(&names.0).is_file() && bundled.join(&names.1).is_file() {
                    (bundled, names.0, names.1)
                } else {
                    (
                        exe_dir.to_path_buf(),
                        "coredns".to_string(),
                        "caddy".to_string(),
                    )
                }
            }
            None => (
                exe_dir.to_path_buf(),
                "coredns".to_string(),
                "caddy".to_string(),
            ),
        },
    };
    let runtime_directory = state_dir.join("runtime");
    Ok(LinuxPaths {
        admin_socket: runtime_directory.join("caddy-admin.sock"),
        coredns_exe: sidecars.join(coredns_name),
        caddy_exe: sidecars.join(caddy_name),
        caddy_data_directory: state_dir.join("caddy-data"),
        runtime_directory,
    })
}

/// CoreDNS listener for readiness probes (unprivileged, mirroring the
/// rendered Corefile port).
pub const LINUX_DNS_PORT: u16 = 5354;
/// Plain-HTTP listener for readiness probes.
pub const LINUX_HTTP_PORT: u16 = 8080;
/// Loopback setup server port (mirrors the core default).
pub const LINUX_SETUP_PORT: u16 = 4781;

pub trait BackendProcessHost: Send + Sync {
    fn spawn(&self, exe: &Path, args: &[String]) -> Result<Box<dyn BackendChild>, String>;
}

pub trait BackendChild: Send {
    fn terminate(&self) -> Result<(), String>;
    fn wait_exit(&self, timeout: Duration) -> Result<u32, String>;
    /// Exit code if the child already finished, `None` while it runs.
    fn try_wait(&self) -> Result<Option<u32>, String>;
}

pub trait BackendReadiness: Send + Sync {
    fn wait_dns(&self, address: &str, port: u16, expected: &str) -> Result<(), String>;
    fn wait_http(&self, address: &str, port: u16, host: &str) -> Result<(), String>;
}

pub trait BackendAdmin: Send + Sync {
    fn load(&self, config: &serde_json::Value) -> Result<(), String>;
}

pub trait BackendSetup: Send + Sync {
    fn start(&self, address: SocketAddr, certificate: PathBuf) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn is_running(&self) -> bool;
}

pub struct LinuxAdapters<'a> {
    pub process_host: &'a dyn BackendProcessHost,
    pub readiness: &'a dyn BackendReadiness,
    pub caddy: &'a dyn BackendAdmin,
    pub setup: &'a dyn BackendSetup,
}

struct BackendState {
    running: bool,
    applied_revision: u64,
}

pub struct LinuxLiveBackend<'a> {
    state_dir: PathBuf,
    paths: LinuxPaths,
    adapters: LinuxAdapters<'a>,
    inspector: Option<super::inspector::LinuxHostInspector>,
    state: Mutex<BackendState>,
    children: Mutex<Vec<Box<dyn BackendChild>>>,
}

impl<'a> LinuxLiveBackend<'a> {
    pub fn with_adapters(paths: LinuxPaths, adapters: LinuxAdapters<'a>) -> Self {
        let state_dir = paths
            .runtime_directory
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| paths.runtime_directory.clone());
        Self {
            state_dir,
            paths,
            adapters,
            inspector: None,
            state: Mutex::new(BackendState {
                running: false,
                applied_revision: 0,
            }),
            children: Mutex::new(Vec::new()),
        }
    }

    /// Attach the host inspector run by `diagnose` after the manifest
    /// gate passes. The factory wires the real one; tests substitute
    /// fakes or omit it.
    pub fn with_inspector(mut self, inspector: super::inspector::LinuxHostInspector) -> Self {
        self.inspector = Some(inspector);
        self
    }

    fn write_text(&self, name: &str, contents: &str) -> Result<PathBuf, RuntimeError> {
        std::fs::create_dir_all(&self.paths.runtime_directory).map_err(failure)?;
        let path = self.paths.runtime_directory.join(name);
        std::fs::write(&path, contents).map_err(failure)?;
        Ok(path)
    }

    /// Terminate every tracked child, best-effort. Used by `stop` and to
    /// avoid leaking sidecars (and holding ports) when `apply` fails.
    fn terminate_all(&self) {
        let mut children = self
            .children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for child in children.drain(..) {
            let _ = child.terminate();
            let _ = child.wait_exit(Duration::from_secs(5));
        }
    }

    /// Every tracked child still running. A retry or settings change must
    /// never stack a second CoreDNS/Caddy beside live ones.
    fn children_alive(&self) -> bool {
        let children = self
            .children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        !children.is_empty()
            && children
                .iter()
                .all(|child| matches!(child.try_wait(), Ok(None)))
    }

    fn manifest_path(&self) -> PathBuf {
        crate::platform::unix::paths::install_manifest_path(&self.state_dir)
    }
}

fn failure(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Failure(error.to_string())
}

impl crate::RuntimeBackend for LinuxLiveBackend<'_> {
    fn apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        // Idempotent retry: a live runtime at this revision needs no new
        // sidecars (mirrors the macOS fast path).
        let (running, applied_revision) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            (state.running, state.applied_revision)
        };
        if running
            && applied_revision == settings.revision
            && self.children_alive()
            && self.adapters.setup.is_running()
        {
            return Ok(ApplyResult {
                desired_revision: settings.revision,
                applied_revision,
                status: ApplyStatus::Applied,
                message: None,
            });
        }
        // Stop-before-spawn: never stack a second CoreDNS/Caddy beside live
        // ones (a timed-out CLI retry must converge, not conflict).
        self.terminate_all();
        let _ = self.adapters.setup.stop();
        let corefile = super::config::render_linux_corefile(settings).map_err(failure)?;
        let corefile_path = self.write_text("Corefile", &corefile)?;
        let proxy = linux_proxy_options(settings, &self.paths.admin_socket).map_err(failure)?;
        let mut config = build_proxy_config(settings, &proxy).map_err(failure)?;
        // File-system storage rooted in the state dir (mirrors Windows):
        // Caddy issues from the CA that setup generated and trusted,
        // instead of a per-user-store CA the trust step never sees.
        config["storage"] = serde_json::json!({
            "module": "file_system",
            "root": self.paths.caddy_data_directory.to_string_lossy(),
        });
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

        // Probe the first plain-HTTP mapping; TLS-only or empty configs
        // skip HTTP readiness.
        let http_host = settings
            .mappings
            .iter()
            .find(|mapping| mapping.enabled && !mapping.tls)
            .map(|mapping| mapping.hostname.clone());
        if let Err(error) = self
            .adapters
            .setup
            .start(
                SocketAddr::from((Ipv4Addr::LOCALHOST, LINUX_SETUP_PORT)),
                self.paths
                    .caddy_data_directory
                    .join("pki/authorities/local/root.crt"),
            )
            .map_err(RuntimeError::Failure)
            .and_then(|()| {
                self.adapters
                    .readiness
                    .wait_dns("127.0.0.1", LINUX_DNS_PORT, &settings.lan_address)
                    .map_err(RuntimeError::Failure)
            })
            .and_then(|()| match &http_host {
                Some(host) => self
                    .adapters
                    .readiness
                    .wait_http("127.0.0.1", LINUX_HTTP_PORT, host)
                    .map_err(RuntimeError::Failure),
                None => Ok(()),
            })
            .and_then(|()| {
                self.adapters
                    .caddy
                    .load(&config)
                    .map_err(RuntimeError::Failure)
            })
        {
            self.terminate_all();
            let _ = self.adapters.setup.stop();
            return Err(error);
        }

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.running = true;
        state.applied_revision = settings.revision;
        drop(state);
        Ok(ApplyResult {
            desired_revision: settings.revision,
            applied_revision: settings.revision,
            status: ApplyStatus::Applied,
            message: None,
        })
    }

    fn stop(&self) -> Result<(), RuntimeError> {
        self.terminate_all();
        let _ = self.adapters.setup.stop();
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

    fn diagnose(&self, settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError> {
        // Manifest gate first (mirrors the skeleton): without a valid
        // manifest there is nothing to inspect against.
        let path = self.manifest_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(vec![setup_required(&path)]);
            }
            Err(error) => {
                return Ok(vec![setup_manifest_invalid(format!(
                    "unable to read {}: {error}",
                    path.display()
                ))]);
            }
        };
        match parse_linux_manifest(&bytes) {
            Ok(_) => Ok(match &self.inspector {
                Some(inspector) => inspector.inspect(settings),
                None => Vec::new(),
            }),
            Err(error) => Ok(vec![setup_manifest_invalid(error.to_string())]),
        }
    }
}

/// Real process host: plain `Command` spawns, no shell and no quoting
/// (Unix argv needs neither, unlike the Windows command-line string).
pub struct SystemProcessHost;

struct SystemChild {
    child: Mutex<std::process::Child>,
}

impl BackendProcessHost for SystemProcessHost {
    fn spawn(&self, exe: &Path, args: &[String]) -> Result<Box<dyn BackendChild>, String> {
        std::process::Command::new(exe)
            .args(args)
            .stdin(std::process::Stdio::null())
            .spawn()
            .map(|child| {
                Box::new(SystemChild {
                    child: Mutex::new(child),
                }) as Box<dyn BackendChild>
            })
            .map_err(|error| format!("cannot launch sidecar: {error}"))
    }
}

impl BackendChild for SystemChild {
    fn terminate(&self) -> Result<(), String> {
        self.child
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .kill()
            .map_err(|error| error.to_string())
    }

    fn wait_exit(&self, timeout: Duration) -> Result<u32, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.try_wait()? {
                Some(code) => return Ok(code),
                None => {
                    if std::time::Instant::now() >= deadline {
                        return Err("sidecar did not exit in time".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    fn try_wait(&self) -> Result<Option<u32>, String> {
        self.child
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .try_wait()
            .map(|status| status.map(|status| status.code().unwrap_or(-1) as u32))
            .map_err(|error| error.to_string())
    }
}

/// Real readiness probes with VM-tolerant timeouts (slow disks and
/// first-run CA generation delay sidecar readiness).
pub struct SystemReadiness;

const READINESS_DEADLINE: Duration = Duration::from_secs(30);
const READINESS_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const READINESS_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Retry `probe` until the deadline; every error is transient until then.
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
        let address: Ipv4Addr = address
            .parse()
            .map_err(|_| format!("invalid probe address: {address}"))?;
        let expected: Ipv4Addr = expected
            .parse()
            .map_err(|_| format!("invalid expected address: {expected}"))?;
        wait_for_probe("CoreDNS", || {
            DnsReadinessProbe::new(address, port, expected, READINESS_ATTEMPT_TIMEOUT).check()
        })
    }

    fn wait_http(&self, address: &str, port: u16, host: &str) -> Result<(), String> {
        let address: Ipv4Addr = address
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

/// Real setup server: the shared in-process HTTP server on 127.0.0.1:4781.
/// The certificate path only matters for the `/ca.crt`-style routes, which
/// resolve once the Linux trust milestone lands; `/` serves regardless.
pub struct SystemSetup {
    server: Mutex<Option<SetupServer>>,
}

impl SystemSetup {
    pub fn new() -> Self {
        Self {
            server: Mutex::new(None),
        }
    }
}

impl Default for SystemSetup {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendSetup for SystemSetup {
    fn start(&self, address: SocketAddr, certificate: PathBuf) -> Result<(), String> {
        let mut server = self
            .server
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if server.is_some() {
            return Ok(());
        }
        let bound = SetupServer::bind(address, certificate)
            .map_err(|error| format!("unable to start the setup server: {error}"))?;
        *server = Some(bound);
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let mut server = self
            .server
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(bound) = server.as_ref() {
            bound
                .shutdown()
                .map_err(|error| format!("unable to stop the setup server: {error}"))?;
        }
        *server = None;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.server
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_some()
    }
}

pub fn runtime_backend(
    state_dir: &Path,
    resources: Option<PathBuf>,
    sidecars: Option<PathBuf>,
    settings_only: bool,
) -> Result<std::sync::Arc<dyn crate::RuntimeBackend>, Box<dyn std::error::Error>> {
    if settings_only {
        return Ok(std::sync::Arc::new(crate::UnavailableRuntimeBackend));
    }
    let current_dir = std::env::current_dir()?;
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or("the daemon executable has no parent directory")?
        .to_path_buf();
    // Installed bundle layout: sidecars live under
    // `<exe-dir>/sidecars/<triple>/` with triple-infixed names. Derive that
    // automatically so GUI/CLI launches, which pass no explicit locations,
    // resolve the staged sidecars. Explicit `--resources`/`--sidecars`
    // overrides still win; dev trees without a bundle fall back to the
    // exe-adjacent plain names in `resolve_paths`.
    let resources = resources.or_else(|| bundled_resources_dir(&exe_dir));
    let paths = resolve_paths(
        state_dir,
        &current_dir,
        &exe_dir,
        resources.as_deref(),
        sidecars,
    )?;
    let admin = CaddyAdminClient::new(paths.admin_socket.clone())?;
    let inspector = super::inspector::LinuxHostInspector::new(
        Box::new(super::super::windows::setup::SystemCommandRunner),
        state_dir.to_path_buf(),
        paths.coredns_exe.clone(),
        paths.caddy_exe.clone(),
    );
    let backend = LinuxLiveBackend::with_adapters(
        paths,
        LinuxAdapters {
            process_host: Box::leak(Box::new(SystemProcessHost)),
            readiness: Box::leak(Box::new(SystemReadiness)),
            caddy: Box::leak(Box::new(admin)),
            setup: Box::leak(Box::new(SystemSetup::new())),
        },
    )
    .with_inspector(inspector);
    Ok(std::sync::Arc::new(backend))
}
