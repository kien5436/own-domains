mod host;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

use std::{path::Path, path::PathBuf, sync::Arc};

pub use host::{
    HostInspector, HostInstaller, SetupAction, SetupApplyResult, SetupFailure, SetupPreview,
    SetupRequest, UninstallRequest,
};

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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state_dir, resources, sidecars, settings_only);
        Ok(Arc::new(crate::UnavailableRuntimeBackend))
    }
}
