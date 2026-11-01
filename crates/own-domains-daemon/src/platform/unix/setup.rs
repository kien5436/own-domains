//! Setup transaction primitives shared by Unix adapters.
//!
//! Host file model and contract, receipt-path backup naming, command
//! builders, and failure construction. Platform orchestration and trust
//! specifics stay with each adapter.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use super::command::{HostCommand, HostCommandOutput};
use super::files::HostFileKind;
use crate::platform::SetupFailure;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupFile {
    pub content: Vec<u8>,
    pub mode: u32,
    pub owner_id: u32,
    pub kind: HostFileKind,
}

pub trait SetupHost: Send + Sync {
    fn inspect_file(&self, path: &Path) -> Result<Option<SetupFile>, String>;
    fn atomic_write(&self, path: &Path, file: &SetupFile) -> Result<(), String>;
    fn write_backup(&self, path: &Path, file: &SetupFile) -> Result<(), String> {
        self.atomic_write(path, file)
    }
    fn remove_file(&self, path: &Path) -> Result<(), String>;
    fn remove_tree(&self, path: &Path) -> Result<(), String>;
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String>;
    fn certificate_trusted(&self, certificate: &Path) -> Result<bool, String>;
    fn pf_enabled(&self) -> Result<bool, String>;
    fn final_probe(&self) -> Result<(), String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostArtifact {
    pub path: PathBuf,
    pub content: String,
    pub mode: u32,
    pub owner_id: u32,
    pub backup_existing: bool,
    pub managed_block: bool,
}

pub fn backup_path(backup_directory: &Path, path: &Path) -> PathBuf {
    backup_directory.join(path.strip_prefix("/").unwrap_or(path))
}

pub fn command<const N: usize>(program: &str, args: [&str; N]) -> HostCommand {
    HostCommand {
        program: program.into(),
        args: args.into_iter().map(String::from).collect(),
        timeout: Duration::from_secs(10),
    }
}

pub fn command_error(command: &HostCommand, output: &HostCommandOutput) -> String {
    let detail = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    format!(
        "{} failed: {}",
        command.program.display(),
        String::from_utf8_lossy(detail)
            .chars()
            .take(240)
            .collect::<String>()
    )
}

pub fn failure(message: impl ToString) -> SetupFailure {
    SetupFailure {
        message: message.to_string(),
        unresolved: vec![],
        exit_code: 4,
    }
}
