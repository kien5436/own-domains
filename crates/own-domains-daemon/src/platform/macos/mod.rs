mod config;
mod factory;
mod host;
mod paths;
mod process;
mod runtime;
mod setup;

pub(crate) use factory::runtime_backend;

pub use crate::platform::{
    HostInspector, HostInstaller, SetupAction, SetupApplyResult, SetupFailure, SetupPreview,
    SetupRequest, UninstallRequest,
};

pub use config::{
    CandidateConfiguration, ConfigurationError, ConfigurationFiles, ConfigurationStager,
    ConfigurationValidator, DEFAULT_DNS_VALIDATION_PORT, SystemConfigurationFiles,
    SystemConfigurationValidator,
};
pub use host::{
    CommandRunner, HostArtifact, HostCommand, HostCommandOutput, HostFileKind, HostFileMetadata,
    HostFileSystem, HostInstallManifest, HostIntegrationInspector, HostSetupError, HostSetupPlan,
    MacOsSetupOptions, ManifestArtifact, SidecarManifest, SidecarSpec, SystemCommandRunner,
    SystemHostFileSystem, build_host_setup_plan, validate_host_setup_plan,
};
pub use paths::{MacOsArchitecture, RuntimePaths};
pub use process::{
    CaddyAdmin, CaddyAdminClient, ChildProcess, DnsReadinessProbe, FileProcessRecords,
    HttpReadinessProbe, ListenerBinding, OwnedProcessRecord, ProcessComponent, ProcessError,
    ProcessHost, ProcessIdentity, ProcessRecords, ProcessState, ProcessSupervisor, ReadinessProbe,
    RetryPolicy, SIGKILL, SIGTERM, SpawnSpec, SystemProcessHost, SystemWaiter, Waiter,
};
pub use runtime::{
    MacOsRuntimeBackend, ReadinessKind, RuntimeAdapters, RuntimeReadiness, SetupServerControl,
    SystemHostInspector, SystemRuntimeReadiness, SystemSetupServer,
};
pub use setup::{
    SetupFile, SetupHost, SystemSetupHost, apply_setup, apply_uninstall, preview_setup,
    preview_uninstall,
};
