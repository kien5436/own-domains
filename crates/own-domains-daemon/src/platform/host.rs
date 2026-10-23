use own_domains_core::{Diagnostic, Settings};
use std::path::PathBuf;

pub trait HostInspector: Send + Sync {
    fn inspect(&self, settings: &Settings) -> Vec<Diagnostic>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub state_directory: PathBuf,
    pub lan_address: String,
    pub interface: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UninstallRequest {
    pub state_directory: PathBuf,
    pub purge: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupAction {
    pub kind: String,
    pub path: PathBuf,
    pub backup_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPreview {
    pub actions: Vec<SetupAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupApplyResult {
    pub written: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
    pub backups: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupFailure {
    pub message: String,
    pub unresolved: Vec<String>,
    pub exit_code: u8,
}

impl SetupFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unresolved: Vec::new(),
            exit_code: 4,
        }
    }

    pub fn with_exit_code(mut self, exit_code: u8) -> Self {
        self.exit_code = exit_code;
        self
    }
}

impl std::fmt::Display for SetupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for SetupFailure {}

pub trait HostInstaller: Send + Sync {
    fn preview_setup(&self, request: &SetupRequest) -> Result<SetupPreview, SetupFailure>;
    fn apply_setup(&self, request: &SetupRequest) -> Result<SetupApplyResult, SetupFailure>;
    fn preview_uninstall(&self, request: &UninstallRequest) -> Result<SetupPreview, SetupFailure>;
    fn apply_uninstall(&self, request: &UninstallRequest)
    -> Result<SetupApplyResult, SetupFailure>;
}
