use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("Own Domains is already running for {path}", path = path.display())]
    AlreadyRunning { path: PathBuf },
    #[error("failed to {operation} {path}: {source}", path = path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid existing IPC path: {path}", path = path.display())]
    InvalidSocketPath { path: PathBuf },
}

pub struct ProcessLock {
    _file: File,
    path: PathBuf,
}

impl ProcessLock {
    pub fn acquire(directory: &Path) -> Result<Self, DaemonError> {
        fs::create_dir_all(directory).map_err(|source| DaemonError::Io {
            operation: "create state directory",
            path: directory.to_path_buf(),
            source,
        })?;
        #[cfg(unix)]
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|source| {
            DaemonError::Io {
                operation: "secure state directory",
                path: directory.to_path_buf(),
                source,
            }
        })?;
        let path = directory.join("own-domains.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&path).map_err(|source| DaemonError::Io {
            operation: "open process lock",
            path: path.clone(),
            source,
        })?;
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            DaemonError::Io {
                operation: "secure process lock",
                path: path.clone(),
                source,
            }
        })?;
        file.try_lock().map_err(|source| match source {
            fs::TryLockError::WouldBlock => DaemonError::AlreadyRunning { path: path.clone() },
            fs::TryLockError::Error(source) => DaemonError::Io {
                operation: "lock state directory",
                path: path.clone(),
                source,
            },
        })?;
        Ok(Self { _file: file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
