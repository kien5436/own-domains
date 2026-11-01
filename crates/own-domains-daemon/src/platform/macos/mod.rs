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

pub use super::{SidecarManifest, SidecarSpec};
pub(crate) use crate::platform::unix::command::MAX_COMMAND_BYTES;
pub use crate::platform::unix::config::{
    CandidateConfiguration, ConfigurationError, ConfigurationFiles, ConfigurationValidator,
    DEFAULT_DNS_VALIDATION_PORT, SystemConfigurationFiles, SystemConfigurationValidator,
};
pub use crate::platform::unix::process::{
    ChildProcess, ListenerBinding, OwnedProcessRecord, ProcessComponent, ProcessError, ProcessHost,
    ProcessIdentity, ProcessRecords, ProcessState, RetryPolicy, SIGKILL, SIGTERM, SpawnSpec,
    SystemProcessHost, SystemWaiter, Waiter,
};
pub use crate::platform::unix::runtime::{
    ReadinessKind, RuntimeAdapters, RuntimeReadiness, SetupServerControl, SystemSetupServer,
};
pub use crate::platform::unix::setup::{HostArtifact, SetupFile, SetupHost};
pub(crate) use crate::platform::unix::setup::{backup_path, command, command_error, failure};
pub use crate::platform::unix::{
    command::{CommandRunner, HostCommand, HostCommandOutput, SystemCommandRunner},
    files::{HostFileKind, HostFileMetadata, HostFileSystem, SystemHostFileSystem},
};
pub use crate::probe::{
    CaddyAdmin, CaddyAdminClient, DnsReadinessProbe, HttpReadinessProbe, ReadinessProbe,
};
pub use config::ConfigurationStager;
pub use host::{
    HostInstallManifest, HostIntegrationInspector, HostSetupError, HostSetupPlan,
    MacOsSetupOptions, ManifestArtifact, NetworkInterface, build_host_setup_plan,
    discover_network_interfaces, validate_host_setup_plan,
};
pub(crate) use host::{
    InstalledHostManifest, parse_host_install_manifest, validate_installed_manifest,
};
pub use paths::{MacOsArchitecture, RuntimePaths};
pub use process::{FileProcessRecords, ProcessSupervisor};
pub use runtime::{MacOsRuntimeBackend, SystemHostInspector, SystemRuntimeReadiness};
pub use setup::{SystemSetupHost, apply_setup, apply_uninstall, preview_setup, preview_uninstall};
