use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{message}")]
    Validation { message: String },
    #[error("settings revision conflict: expected {expected}, found {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("hostname is already registered: {hostname}")]
    DuplicateHostname { hostname: String },
    #[error("settings are corrupt at {path}: {message}", path = path.display())]
    CorruptSettings { path: PathBuf, message: String },
    #[error("failed to {operation} {path}: {source}", path = path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type CoreResult<T> = Result<T, CoreError>;
