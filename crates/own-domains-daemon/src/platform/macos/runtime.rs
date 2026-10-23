use own_domains_core::{ApplyResult, ApplyStatus, Diagnostic, DiagnosticStatus, Settings};
use own_domains_protocol::RuntimeStatus;
use serde_json::Value;
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};

use crate::{RuntimeBackend, RuntimeError, SetupServer};
use crate::platform::HostInspector;

use super::{
    CaddyAdmin, CaddyAdminClient, ConfigurationError, ConfigurationFiles, ConfigurationStager,
    ConfigurationValidator, DnsReadinessProbe, FileProcessRecords, HostIntegrationInspector,
    HttpReadinessProbe, ProcessComponent, ProcessError, ProcessHost, ProcessRecords, ProcessState,
    ProcessSupervisor, ReadinessProbe, RuntimePaths, SidecarManifest, SystemCommandRunner,
    SystemConfigurationFiles, SystemConfigurationValidator, SystemHostFileSystem,
    SystemProcessHost, SystemWaiter, Waiter,
};

pub struct SystemHostInspector {
    paths: RuntimePaths,
    sidecars: SidecarManifest,
    files: Box<dyn super::HostFileSystem>,
    commands: Box<dyn super::CommandRunner>,
}

impl SystemHostInspector {
    pub fn new(paths: RuntimePaths, sidecars: SidecarManifest) -> Self {
        Self {
            paths,
            sidecars,
            files: Box::new(SystemHostFileSystem),
            commands: Box::new(SystemCommandRunner),
        }
    }
}

impl HostInspector for SystemHostInspector {
    fn inspect(&self, settings: &Settings) -> Vec<Diagnostic> {
        HostIntegrationInspector::new(
            &self.paths,
            &self.sidecars,
            self.files.as_ref(),
            self.commands.as_ref(),
        )
        .inspect(settings)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessKind {
    Dns,
    CaddyAdmin,
    Setup,
    Http,
}

impl ReadinessKind {
    fn component(self) -> &'static str {
        match self {
            Self::Dns => "CoreDNS",
            Self::CaddyAdmin => "Caddy admin",
            Self::Setup => "setup server",
            Self::Http => "Caddy HTTP",
        }
    }
}

pub trait RuntimeReadiness: Send + Sync {
    fn check(&self, kind: ReadinessKind, settings: &Settings) -> Result<(), String>;
}

pub struct SystemRuntimeReadiness {
    socket: PathBuf,
    timeout: Duration,
}

impl SystemRuntimeReadiness {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            socket: paths.runtime_directory.join("caddy-admin.sock"),
            timeout: Duration::from_secs(1),
        }
    }
}

impl RuntimeReadiness for SystemRuntimeReadiness {
    fn check(&self, kind: ReadinessKind, settings: &Settings) -> Result<(), String> {
        match kind {
            ReadinessKind::Dns => {
                let lan = settings.lan_address.parse::<Ipv4Addr>().map_err(|_| {
                    "a valid LAN IPv4 address is required for the DNS probe".to_string()
                })?;
                DnsReadinessProbe::new(Ipv4Addr::LOCALHOST, 5354, lan, self.timeout).check()
            }
            ReadinessKind::CaddyAdmin => CaddyAdminClient::new(self.socket.clone())?.is_ready(),
            ReadinessKind::Setup => HttpReadinessProbe::new(
                Ipv4Addr::LOCALHOST,
                4781,
                "127.0.0.1",
                "/fingerprint",
                200,
                self.timeout,
            )
            .check(),
            ReadinessKind::Http => HttpReadinessProbe::new(
                Ipv4Addr::LOCALHOST,
                8080,
                "setup.own",
                "/",
                200,
                self.timeout,
            )
            .check(),
        }
    }
}

struct RuntimeProbe<'a> {
    readiness: &'a dyn RuntimeReadiness,
    kind: ReadinessKind,
    settings: &'a Settings,
}

impl ReadinessProbe for RuntimeProbe<'_> {
    fn component(&self) -> &'static str {
        self.kind.component()
    }

    fn check(&self) -> Result<(), String> {
        self.readiness.check(self.kind, self.settings)
    }
}

pub trait SetupServerControl: Send + Sync {
    fn start(&self, address: SocketAddr, certificate: PathBuf) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn is_running(&self) -> bool;
}

pub struct SystemSetupServer {
    server: Mutex<Option<SetupServer>>,
}

impl SystemSetupServer {
    pub fn new() -> Self {
        Self {
            server: Mutex::new(None),
        }
    }
}

impl Default for SystemSetupServer {
    fn default() -> Self {
        Self::new()
    }
}

impl SetupServerControl for SystemSetupServer {
    fn start(&self, address: SocketAddr, certificate: PathBuf) -> Result<(), String> {
        let mut server = self.server.lock().unwrap();
        if server.is_some() {
            return Ok(());
        }
        let bound = SetupServer::bind(address, certificate)
            .map_err(|error| format!("unable to start the setup server: {error}"))?;
        *server = Some(bound);
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let mut guard = self.server.lock().unwrap();
        if let Some(server) = guard.as_ref() {
            server
                .shutdown()
                .map_err(|error| format!("unable to stop the setup server: {error}"))?;
        }
        *guard = None;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.server.lock().unwrap().is_some()
    }
}

#[derive(Clone, Copy)]
pub struct RuntimeAdapters<'a> {
    pub inspector: &'a dyn HostInspector,
    pub files: &'a dyn ConfigurationFiles,
    pub validator: &'a dyn ConfigurationValidator,
    pub process_host: &'a dyn ProcessHost,
    pub records: &'a dyn ProcessRecords,
    pub waiter: &'a dyn Waiter,
    pub caddy: &'a dyn CaddyAdmin,
    pub setup: &'a dyn SetupServerControl,
    pub readiness: &'a dyn RuntimeReadiness,
}

#[derive(Default)]
struct RuntimeState {
    active: bool,
    applied_revision: u64,
    last_error: Option<String>,
}

pub struct MacOsRuntimeBackend<'a> {
    paths: RuntimePaths,
    adapters: RuntimeAdapters<'a>,
    supervisor: ProcessSupervisor<'a>,
    state: Mutex<RuntimeState>,
}

impl MacOsRuntimeBackend<'static> {
    pub fn new(paths: RuntimePaths, sidecars: SidecarManifest) -> Self {
        Self::try_new(paths, sidecars).expect("the macOS runtime adapter paths are valid")
    }

    pub fn try_new(paths: RuntimePaths, sidecars: SidecarManifest) -> Result<Self, RuntimeError> {
        let inspector: &'static SystemHostInspector =
            Box::leak(Box::new(SystemHostInspector::new(paths.clone(), sidecars)));
        let files: &'static SystemConfigurationFiles =
            Box::leak(Box::new(SystemConfigurationFiles));
        let validator: &'static SystemConfigurationValidator =
            Box::leak(Box::new(SystemConfigurationValidator));
        let process_host: &'static SystemProcessHost = Box::leak(Box::new(SystemProcessHost));
        let records: &'static FileProcessRecords =
            Box::leak(Box::new(FileProcessRecords::new(&paths)));
        let waiter: &'static SystemWaiter = Box::leak(Box::new(SystemWaiter));
        let caddy = Box::leak(Box::new(
            CaddyAdminClient::new(paths.runtime_directory.join("caddy-admin.sock"))
                .map_err(|message| failure(format!("Caddy admin socket is invalid: {message}")))?,
        ));
        let setup: &'static SystemSetupServer = Box::leak(Box::new(SystemSetupServer::new()));
        let readiness: &'static SystemRuntimeReadiness =
            Box::leak(Box::new(SystemRuntimeReadiness::new(&paths)));
        let adapters: &'static RuntimeAdapters<'static> = Box::leak(Box::new(RuntimeAdapters {
            inspector,
            files,
            validator,
            process_host,
            records,
            waiter,
            caddy,
            setup,
            readiness,
        }));
        Ok(Self::with_adapters(paths, *adapters))
    }
}

impl<'a> MacOsRuntimeBackend<'a> {
    pub fn with_adapters(paths: RuntimePaths, adapters: RuntimeAdapters<'a>) -> Self {
        let supervisor = ProcessSupervisor::new(
            paths.clone(),
            adapters.process_host,
            adapters.records,
            adapters.waiter,
        );
        Self {
            paths,
            adapters,
            supervisor,
            state: Mutex::new(RuntimeState::default()),
        }
    }

    fn stager(&self) -> ConfigurationStager<'_> {
        ConfigurationStager::new(&self.paths, self.adapters.files, self.adapters.validator)
    }

    fn start_fresh(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        let stager = self.stager();
        let candidate = stager.stage(settings).map_err(configuration_error)?;
        stager.validate(&candidate).map_err(configuration_error)?;

        self.adapters
            .setup
            .start(setup_address(), self.paths.ca_certificate.clone())
            .map_err(|message| failure(format!("setup server failed: {message}")))?;

        let mut coredns_started = false;
        let mut caddy_started = false;
        let outcome = (|| -> Result<(), RuntimeError> {
            self.supervisor
                .start(ProcessComponent::CoreDns, &candidate.corefile_path)
                .map_err(process_error)?;
            coredns_started = true;
            self.supervisor
                .start(ProcessComponent::Caddy, &candidate.proxy_path)
                .map_err(process_error)?;
            caddy_started = true;
            self.wait_ready(ReadinessKind::Dns, settings)?;
            self.wait_ready(ReadinessKind::CaddyAdmin, settings)?;
            self.wait_ready(ReadinessKind::Setup, settings)?;
            self.wait_ready(ReadinessKind::Http, settings)?;
            stager.promote(&candidate).map_err(configuration_error)?;
            Ok(())
        })();

        match outcome {
            Ok(()) => {
                let mut state = self.state.lock().unwrap();
                state.active = true;
                state.applied_revision = candidate.revision;
                state.last_error = None;
                Ok(applied_result(candidate.revision, candidate.revision))
            }
            Err(error) => {
                if caddy_started {
                    let _ = self.supervisor.stop_component(ProcessComponent::Caddy);
                }
                if coredns_started {
                    let _ = self.supervisor.stop_component(ProcessComponent::CoreDns);
                }
                let _ = self.adapters.setup.stop();
                let mut state = self.state.lock().unwrap();
                state.active = false;
                state.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn reconcile(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        let stager = self.stager();
        let candidate = stager.stage(settings).map_err(configuration_error)?;
        stager.validate(&candidate).map_err(configuration_error)?;
        let caddy_running = is_running(
            &self
                .supervisor
                .inspect_component(ProcessComponent::Caddy)
                .map_err(process_error)?,
        );
        let setup_started = if self.adapters.setup.is_running() {
            false
        } else {
            self.adapters
                .setup
                .start(setup_address(), self.paths.ca_certificate.clone())
                .map_err(|message| failure(format!("setup server failed: {message}")))?;
            true
        };

        if caddy_running {
            if let Err(message) = self.adapters.caddy.load(&candidate.proxy) {
                self.stop_new_setup_server(setup_started);
                return Err(failure(format!(
                    "Caddy rejected the candidate configuration: {message}"
                )));
            }
        } else {
            let _ = self.supervisor.stop_component(ProcessComponent::Caddy);
            if let Err(error) = self
                .supervisor
                .start(ProcessComponent::Caddy, &candidate.proxy_path)
            {
                self.stop_new_setup_server(setup_started);
                return Err(process_error(error));
            }
            if let Err(error) = self.wait_ready(ReadinessKind::CaddyAdmin, settings) {
                let _ = self.supervisor.stop_component(ProcessComponent::Caddy);
                self.stop_new_setup_server(setup_started);
                return Err(error);
            }
        }

        let outcome = (|| -> Result<(), RuntimeError> {
            if self
                .adapters
                .records
                .read(ProcessComponent::CoreDns)
                .map_err(record_error)?
                .is_some()
            {
                self.supervisor
                    .stop_component(ProcessComponent::CoreDns)
                    .map_err(process_error)?;
            }
            self.supervisor
                .start(ProcessComponent::CoreDns, &candidate.corefile_path)
                .map_err(process_error)?;
            self.wait_ready(ReadinessKind::Dns, settings)?;
            stager.promote(&candidate).map_err(configuration_error)?;
            Ok(())
        })();

        match outcome {
            Ok(()) => {
                let mut state = self.state.lock().unwrap();
                state.active = true;
                state.applied_revision = candidate.revision;
                state.last_error = None;
                Ok(applied_result(candidate.revision, candidate.revision))
            }
            Err(error) => {
                let restoration =
                    self.restore_previous_runtime(settings, caddy_running, setup_started);
                let mut state = self.state.lock().unwrap();
                state.active = true;
                state.last_error = Some(format!("{error}; {restoration}"));
                Err(failure(format!("{error}; {restoration}")))
            }
        }
    }

    fn stop_new_setup_server(&self, started: bool) {
        if started {
            let _ = self.adapters.setup.stop();
        }
    }

    fn restore_previous_runtime(
        &self,
        settings: &Settings,
        caddy_was_running: bool,
        setup_was_started: bool,
    ) -> String {
        let stager = self.stager();
        let mut failures = Vec::new();

        if caddy_was_running {
            match self.adapters.files.read(&stager.applied_proxy()) {
                Ok(Some(bytes)) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(config) => {
                        if let Err(error) = self.adapters.caddy.load(&config) {
                            failures.push(format!("Caddy restore failed: {error}"));
                        }
                    }
                    Err(error) => {
                        failures.push(format!("Caddy restore config is invalid: {error}"))
                    }
                },
                Ok(None) => failures.push("Caddy restore config is missing".into()),
                Err(error) => failures.push(format!("Caddy restore config is unreadable: {error}")),
            }
        } else if let Err(error) = self.supervisor.stop_component(ProcessComponent::Caddy) {
            failures.push(format!("Caddy rollback failed: {error}"));
        }

        if let Err(error) = self.supervisor.stop_component(ProcessComponent::CoreDns) {
            failures.push(format!("CoreDNS rollback stop failed: {error}"));
        }
        match self.adapters.files.read(&stager.applied_corefile()) {
            Ok(Some(_)) => {
                let corefile = stager.applied_corefile();
                if let Err(error) = self.supervisor.start(ProcessComponent::CoreDns, &corefile) {
                    failures.push(format!("CoreDNS restore failed: {error}"));
                } else if let Err(error) = self.wait_ready(ReadinessKind::Dns, settings) {
                    failures.push(format!("CoreDNS restore readiness failed: {error}"));
                }
            }
            Ok(None) => failures.push("CoreDNS restore config is missing".into()),
            Err(error) => failures.push(format!("CoreDNS restore config is unreadable: {error}")),
        }

        if setup_was_started && let Err(error) = self.adapters.setup.stop() {
            failures.push(format!("setup server rollback failed: {error}"));
        }

        if failures.is_empty() {
            "the previous configuration was restored".into()
        } else {
            format!("restore incomplete: {}", failures.join("; "))
        }
    }

    fn wait_ready(&self, kind: ReadinessKind, settings: &Settings) -> Result<(), RuntimeError> {
        let probe = RuntimeProbe {
            readiness: self.adapters.readiness,
            kind,
            settings,
        };
        self.supervisor
            .wait_for_readiness(&probe)
            .map_err(process_error)
    }

    fn clean_children(&self) -> Result<(), RuntimeError> {
        for component in [ProcessComponent::Caddy, ProcessComponent::CoreDns] {
            match self
                .supervisor
                .inspect_component(component)
                .map_err(process_error)?
            {
                ProcessState::Stopped => {}
                ProcessState::Running(_) | ProcessState::Stale { .. } => {
                    self.supervisor
                        .stop_component(component)
                        .map_err(process_error)?;
                }
                ProcessState::Ambiguous { record, reason } => {
                    return Err(failure(format!(
                        "ambiguous {} process (pid {}): {reason}",
                        component.name(),
                        record.pid
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_host(&self, settings: &Settings) -> Result<(), RuntimeError> {
        let owned = self.owned_records_present()?;
        let diagnostics = self.adapters.inspector.inspect(settings);
        let blocking = diagnostics
            .into_iter()
            .filter(|diagnostic| {
                diagnostic.status == DiagnosticStatus::Error
                    && !(owned && diagnostic.code == "portConflict")
            })
            .collect::<Vec<_>>();
        if blocking.is_empty() {
            return Ok(());
        }
        Err(failure(
            blocking
                .iter()
                .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }

    fn owned_records_present(&self) -> Result<bool, RuntimeError> {
        Ok(self
            .adapters
            .records
            .read(ProcessComponent::CoreDns)
            .map_err(record_error)?
            .is_some()
            || self
                .adapters
                .records
                .read(ProcessComponent::Caddy)
                .map_err(record_error)?
                .is_some())
    }

    fn children_running(&self) -> Result<bool, RuntimeError> {
        let coredns = self
            .supervisor
            .inspect_component(ProcessComponent::CoreDns)
            .map_err(process_error)?;
        let caddy = self
            .supervisor
            .inspect_component(ProcessComponent::Caddy)
            .map_err(process_error)?;
        Ok(is_running(&coredns) && is_running(&caddy))
    }
}

impl RuntimeBackend for MacOsRuntimeBackend<'_> {
    fn apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        self.check_host(settings)?;
        let (active, applied_revision) = {
            let state = self.state.lock().unwrap();
            (state.active, state.applied_revision)
        };
        if active
            && applied_revision == settings.revision
            && self.children_running()?
            && self.adapters.setup.is_running()
        {
            return Ok(applied_result(settings.revision, applied_revision));
        }
        if active {
            return self.reconcile(settings);
        }
        self.clean_children()?;
        if self.adapters.setup.is_running() {
            self.adapters
                .setup
                .stop()
                .map_err(|message| failure(format!("setup server failed: {message}")))?;
        }
        self.start_fresh(settings)
    }

    fn stop(&self) -> Result<(), RuntimeError> {
        {
            let mut state = self.state.lock().unwrap();
            state.active = false;
        }
        let mut failures = Vec::new();
        if let Err(error) = self.adapters.setup.stop() {
            failures.push(format!("setup server: {error}"));
        }
        for component in [ProcessComponent::Caddy, ProcessComponent::CoreDns] {
            if let Err(error) = self.supervisor.stop_component(component) {
                failures.push(error.to_string());
            }
        }
        let mut state = self.state.lock().unwrap();
        if failures.is_empty() {
            state.last_error = None;
            Ok(())
        } else {
            let message = failures.join("; ");
            state.last_error = Some(message.clone());
            Err(failure(message))
        }
    }

    fn inspect(&self) -> Result<RuntimeStatus, RuntimeError> {
        let coredns = self
            .supervisor
            .inspect_component(ProcessComponent::CoreDns)
            .map_err(process_error)?;
        let caddy = self
            .supervisor
            .inspect_component(ProcessComponent::Caddy)
            .map_err(process_error)?;
        if is_running(&coredns) && is_running(&caddy) {
            return Ok(if self.adapters.setup.is_running() {
                RuntimeStatus::Running
            } else {
                RuntimeStatus::Degraded
            });
        }
        if matches!(coredns, ProcessState::Stopped)
            && matches!(caddy, ProcessState::Stopped)
            && !self.adapters.setup.is_running()
        {
            return Ok(RuntimeStatus::Stopped);
        }
        if matches!(coredns, ProcessState::Ambiguous { .. })
            || matches!(caddy, ProcessState::Ambiguous { .. })
        {
            return Ok(RuntimeStatus::Error);
        }
        Ok(RuntimeStatus::Error)
    }

    fn diagnose(&self, settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError> {
        let mut diagnostics = self.adapters.inspector.inspect(settings);
        let active = self.state.lock().unwrap().active;
        if self.children_running()? {
            diagnostics.retain(|diagnostic| diagnostic.code != "portConflict");
        }
        let mut any_running = false;
        for component in [ProcessComponent::CoreDns, ProcessComponent::Caddy] {
            match self
                .supervisor
                .inspect_component(component)
                .map_err(process_error)?
            {
                ProcessState::Stopped => {}
                ProcessState::Running(record) => {
                    any_running = true;
                    if !active {
                        diagnostics.push(runtime_diagnostic(
                            "staleOwnedChild",
                            format!(
                                "{} is running (pid {}) without an active daemon runtime",
                                component.name(),
                                record.pid
                            ),
                        ));
                    }
                }
                ProcessState::Stale { record, reason } => diagnostics.push(runtime_diagnostic(
                    "staleOwnedChild",
                    format!(
                        "{} ownership record with pid {} is stale: {reason}",
                        component.name(),
                        record.pid
                    ),
                )),
                ProcessState::Ambiguous { record, reason } => {
                    diagnostics.push(runtime_diagnostic(
                        "ambiguousOwnedProcess",
                        format!(
                            "{} process with pid {} cannot be verified: {reason}",
                            component.name(),
                            record.pid
                        ),
                    ));
                }
            }
        }
        if (active || any_running) && !self.adapters.setup.is_running() {
            diagnostics.push(runtime_diagnostic(
                "setupServerUnavailable",
                "the setup server is not running while runtime processes are present".into(),
            ));
        }
        Ok(diagnostics)
    }
}

fn is_running(state: &ProcessState) -> bool {
    matches!(state, ProcessState::Running(_))
}

fn setup_address() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 4781))
}

fn applied_result(desired_revision: u64, applied_revision: u64) -> ApplyResult {
    ApplyResult {
        desired_revision,
        applied_revision,
        status: ApplyStatus::Applied,
        message: None,
    }
}

fn failure(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Failure(message.into())
}

fn configuration_error(error: ConfigurationError) -> RuntimeError {
    failure(error.to_string())
}

fn process_error(error: ProcessError) -> RuntimeError {
    failure(error.to_string())
}

fn record_error(message: String) -> RuntimeError {
    failure(format!("process record: {message}"))
}

fn runtime_diagnostic(code: &str, message: String) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        status: DiagnosticStatus::Error,
        message,
        remedy: Some("stop the runtime and retry".into()),
    }
}
