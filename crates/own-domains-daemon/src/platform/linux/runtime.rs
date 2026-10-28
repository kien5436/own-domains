//! Linux runtime backend skeleton for Milestone 1.
//!
//! The manifest gate only: `diagnose` reports `setupRequired` when the state
//! directory has no install manifest and `setupManifestInvalid` when it
//! cannot be read, parsed, or validated. No processes are started; the full
//! runtime lands in Milestone 2.

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::Arc;

use own_domains_core::{ApplyResult, Diagnostic, DiagnosticStatus, Settings};
use own_domains_protocol::RuntimeStatus;

use super::manifest::parse_linux_manifest;
#[cfg(target_os = "linux")]
use crate::UnavailableRuntimeBackend;
use crate::{RuntimeBackend, RuntimeError};

pub struct LinuxRuntimeBackend {
    state_dir: PathBuf,
}

impl LinuxRuntimeBackend {
    pub fn new(state_dir: &Path) -> Result<Self, RuntimeError> {
        let state_dir = if state_dir.is_absolute() {
            state_dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| RuntimeError::Failure(error.to_string()))?
                .join(state_dir)
        };
        Ok(Self { state_dir })
    }

    fn manifest_path(&self) -> PathBuf {
        self.state_dir.join("install-manifest.json")
    }
}

fn setup_required(path: &Path) -> Diagnostic {
    Diagnostic {
        code: "setupRequired".into(),
        status: DiagnosticStatus::Error,
        message: format!(
            "Linux host setup has not run ({} is missing)",
            path.display()
        ),
        remedy: Some("run setup --apply to install the Linux host integration".into()),
    }
}

fn setup_manifest_invalid(message: String) -> Diagnostic {
    Diagnostic {
        code: "setupManifestInvalid".into(),
        status: DiagnosticStatus::Error,
        message,
        remedy: Some("re-run setup --apply to regenerate the install manifest".into()),
    }
}

impl RuntimeBackend for LinuxRuntimeBackend {
    fn apply(&self, _settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        Err(RuntimeError::Unavailable(
            "the Linux runtime is not implemented yet".into(),
        ))
    }

    fn stop(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn inspect(&self) -> Result<RuntimeStatus, RuntimeError> {
        Ok(RuntimeStatus::Stopped)
    }

    fn diagnose(&self, _settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError> {
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
            Ok(_) => Ok(Vec::new()),
            Err(error) => Ok(vec![setup_manifest_invalid(error.to_string())]),
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn runtime_backend(
    state_dir: &Path,
    _resources: Option<PathBuf>,
    _sidecars: Option<PathBuf>,
    settings_only: bool,
) -> Result<Arc<dyn RuntimeBackend>, Box<dyn std::error::Error>> {
    if settings_only {
        return Ok(Arc::new(UnavailableRuntimeBackend));
    }
    Ok(Arc::new(LinuxRuntimeBackend::new(state_dir)?))
}
