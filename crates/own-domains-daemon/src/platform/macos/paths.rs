use std::path::{Path, PathBuf};

use super::HostSetupError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacOsArchitecture {
    Arm64,
    X86_64,
}

impl MacOsArchitecture {
    fn target(self) -> &'static str {
        match self {
            Self::Arm64 => "aarch64-apple-darwin",
            Self::X86_64 => "x86_64-apple-darwin",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub state_directory: PathBuf,
    pub resource_directory: PathBuf,
    pub backup_directory: PathBuf,
    pub install_manifest: PathBuf,
    pub runtime_directory: PathBuf,
    pub candidate_directory: PathBuf,
    pub applied_directory: PathBuf,
    pub socket_path: PathBuf,
    pub log_directory: PathBuf,
    pub caddy_data_directory: PathBuf,
    pub ca_certificate: PathBuf,
    pub coredns_binary: PathBuf,
    pub caddy_binary: PathBuf,
}

impl RuntimePaths {
    pub fn new(
        state_directory: PathBuf,
        resource_directory: PathBuf,
        backup_directory: PathBuf,
        architecture: MacOsArchitecture,
    ) -> Result<Self, HostSetupError> {
        for (label, path) in [
            ("state directory", state_directory.as_path()),
            ("resource directory", resource_directory.as_path()),
            ("backup directory", backup_directory.as_path()),
        ] {
            if !path.is_absolute() {
                return Err(HostSetupError::RelativePath {
                    label,
                    path: path.to_path_buf(),
                });
            }
        }
        let runtime_directory = state_directory.join("runtime");
        let caddy_data_directory = state_directory.join("caddy-data");
        let target = architecture.target();

        Ok(Self {
            install_manifest: state_directory.join("install-manifest.json"),
            candidate_directory: runtime_directory.join("candidate"),
            applied_directory: runtime_directory.join("applied"),
            socket_path: runtime_directory.join("daemon.sock"),
            log_directory: state_directory.join("logs"),
            ca_certificate: caddy_data_directory.join("pki/authorities/local/root.crt"),
            coredns_binary: resource_directory.join(format!("coredns-{target}")),
            caddy_binary: resource_directory.join(format!("caddy-{target}")),
            state_directory,
            resource_directory,
            backup_directory,
            runtime_directory,
            caddy_data_directory,
        })
    }

    pub fn all(&self) -> Vec<&Path> {
        vec![
            &self.state_directory,
            &self.resource_directory,
            &self.backup_directory,
            &self.install_manifest,
            &self.runtime_directory,
            &self.candidate_directory,
            &self.applied_directory,
            &self.socket_path,
            &self.log_directory,
            &self.caddy_data_directory,
            &self.ca_certificate,
            &self.coredns_binary,
            &self.caddy_binary,
        ]
    }
}
