mod host;

#[cfg(target_os = "macos")]
pub mod macos;

pub use host::{
    HostInspector, HostInstaller, SetupAction, SetupApplyResult, SetupFailure, SetupPreview,
    SetupRequest, UninstallRequest,
};
