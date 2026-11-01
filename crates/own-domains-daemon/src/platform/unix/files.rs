//! Host filesystem access shared by Unix adapters.
//!
//! Symlink-aware metadata, bounded reads, and streaming hashes. No shell.

use std::path::Path;

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostFileKind {
    Regular,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFileMetadata {
    pub kind: HostFileKind,
    pub owner_id: u32,
    pub mode: u32,
}

pub trait HostFileSystem: Send + Sync {
    fn metadata(&self, path: &Path) -> Result<Option<HostFileMetadata>, String>;
    fn read(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, String>;
    fn sha256(&self, path: &Path, max_bytes: u64) -> Result<String, String>;
}

pub struct SystemHostFileSystem;

impl HostFileSystem for SystemHostFileSystem {
    fn metadata(&self, path: &Path) -> Result<Option<HostFileMetadata>, String> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("unable to inspect {}: {error}", path.display()));
            }
        };
        let file_type = metadata.file_type();
        let kind = if file_type.is_file() {
            HostFileKind::Regular
        } else if file_type.is_symlink() {
            HostFileKind::Symlink
        } else {
            HostFileKind::Other
        };
        #[cfg(unix)]
        let (owner_id, mode) = {
            use std::os::unix::fs::MetadataExt;
            (metadata.uid(), metadata.mode() & 0o7777)
        };
        #[cfg(not(unix))]
        let (owner_id, mode) = (0, 0);
        Ok(Some(HostFileMetadata {
            kind,
            owner_id,
            mode,
        }))
    }

    fn read(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
        let mut bytes = Vec::new();
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
        if bytes.len() > max_bytes {
            return Err(format!(
                "{} exceeds the {max_bytes}-byte read limit",
                path.display()
            ));
        }
        Ok(bytes)
    }

    fn sha256(&self, path: &Path, max_bytes: u64) -> Result<String, String> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|error| format!("unable to hash {}: {error}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 16 * 1024];
        let mut reader = file.take(max_bytes + 1);
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("unable to hash {}: {error}", path.display()))?;
            if read == 0 {
                break;
            }
            bytes += read as u64;
            if bytes > max_bytes {
                return Err(format!(
                    "{} exceeds the {max_bytes}-byte hash limit",
                    path.display()
                ));
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}
