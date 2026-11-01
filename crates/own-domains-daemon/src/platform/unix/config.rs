//! Configuration staging and validation shared by Unix adapters.
//!
//! Candidate configurations, validators, and the system file/runner
//! implementations. Staging orchestration stays platform-specific until
//! the shared runtime paths land.

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use super::command::HostCommand;

pub const DEFAULT_DNS_VALIDATION_PORT: u16 = 15354;

#[derive(Clone, Debug)]
pub struct CandidateConfiguration {
    pub revision: u64,
    pub corefile: String,
    pub proxy: Value,
    pub corefile_path: PathBuf,
    pub proxy_path: PathBuf,
    pub validation_corefile_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("invalid configuration: {0}")]
    Validation(String),
    #[error("configuration validation failed: {0}")]
    ValidationFailure(String),
    #[error("configuration promotion failed: {0}")]
    Promotion(String),
}

pub trait ConfigurationFiles: Send + Sync {
    fn create_dir_all(&self, path: &Path) -> Result<(), String>;
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String>;
    fn write_atomic(&self, path: &Path, contents: &[u8], mode: u32) -> Result<(), String>;
    fn rename(&self, from: &Path, to: &Path) -> Result<(), String>;
    fn remove_file(&self, path: &Path) -> Result<(), String>;
    fn remove_tree(&self, path: &Path) -> Result<(), String>;
}

pub trait ConfigurationValidator: Send + Sync {
    fn run(&self, command: &HostCommand) -> Result<(), String>;
    fn expect_running(&self, command: &HostCommand, settle: Duration) -> Result<(), String>;
}

pub struct SystemConfigurationFiles;

impl ConfigurationFiles for SystemConfigurationFiles {
    fn create_dir_all(&self, path: &Path) -> Result<(), String> {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("unable to create {}: {error}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("unable to protect {}: {error}", path.display()))?;
        }
        Ok(())
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("unable to read {}: {error}", path.display())),
        }
    }

    #[cfg_attr(not(unix), allow(unused_variables))]
    fn write_atomic(&self, path: &Path, contents: &[u8], mode: u32) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            self.create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(mode)
                .open(&temporary)
                .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
            file.write_all(contents)
                .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
            file.sync_all()
                .map_err(|error| format!("unable to flush {}: {error}", temporary.display()))?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&temporary, contents)
                .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
        }
        std::fs::rename(&temporary, path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("unable to publish {}: {error}", path.display())
        })
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(from, to)
            .map_err(|error| format!("unable to rename {}: {error}", from.display()))
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("unable to remove {}: {error}", path.display())),
        }
    }

    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("unable to remove {}: {error}", path.display())),
        }
    }
}

pub struct SystemConfigurationValidator;

impl ConfigurationValidator for SystemConfigurationValidator {
    fn run(&self, command: &HostCommand) -> Result<(), String> {
        use std::{
            io::Read,
            process::{Command, Stdio},
            time::Instant,
        };

        let mut child = Command::new(&command.program)
            .args(&command.args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("unable to run {}: {error}", command.program.display()))?;
        let deadline = Instant::now() + command.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    if let Some(mut pipe) = child.stdout.take() {
                        pipe.read_to_end(&mut stdout)
                            .map_err(|error| error.to_string())?;
                    }
                    if let Some(mut pipe) = child.stderr.take() {
                        pipe.read_to_end(&mut stderr)
                            .map_err(|error| error.to_string())?;
                    }
                    if status.success() {
                        return Ok(());
                    }
                    let detail = if stderr.is_empty() { &stdout } else { &stderr };
                    return Err(String::from_utf8_lossy(detail)
                        .chars()
                        .take(240)
                        .collect::<String>());
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} did not finish within {:?}",
                        command.program.display(),
                        command.timeout
                    ));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "unable to wait for {}: {error}",
                        command.program.display()
                    ));
                }
            }
        }
    }

    fn expect_running(&self, command: &HostCommand, settle: Duration) -> Result<(), String> {
        use std::{
            process::{Command, Stdio},
            time::Instant,
        };

        let mut child = Command::new(&command.program)
            .args(&command.args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("unable to start {}: {error}", command.program.display()))?;
        let deadline = Instant::now() + settle;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!(
                        "{} exited during validation with status {status}",
                        command.program.display()
                    ));
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(());
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "unable to wait for {}: {error}",
                        command.program.display()
                    ));
                }
            }
        }
    }
}
