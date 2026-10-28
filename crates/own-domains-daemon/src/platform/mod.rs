mod host;
mod sidecar;

pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

pub mod windows;

use std::{path::Path, path::PathBuf, sync::Arc};

pub use host::{
    HostInspector, HostInstaller, SetupAction, SetupApplyResult, SetupFailure, SetupPreview,
    SetupRequest, UninstallRequest,
};
pub use sidecar::{SidecarManifest, SidecarSpec};

pub fn runtime_backend(
    state_dir: &Path,
    resources: Option<PathBuf>,
    sidecars: Option<PathBuf>,
    settings_only: bool,
) -> Result<Arc<dyn crate::RuntimeBackend>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        macos::runtime_backend(state_dir, resources, sidecars, settings_only)
    }
    #[cfg(target_os = "windows")]
    {
        windows::runtime_backend(state_dir, resources, sidecars, settings_only)
    }
    #[cfg(target_os = "linux")]
    {
        linux::runtime_backend(state_dir, resources, sidecars, settings_only)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (state_dir, resources, sidecars, settings_only);
        Ok(Arc::new(crate::UnavailableRuntimeBackend))
    }
}
