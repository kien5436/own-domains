use own_domains_core::{ApplyResult, Diagnostic, Settings};
use own_domains_protocol::RuntimeStatus;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("runtime is unavailable: {0}")]
    Unavailable(String),
    #[error("runtime operation failed: {0}")]
    Failure(String),
}

pub trait RuntimeBackend: Send + Sync {
    fn apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError>;
    fn stop(&self) -> Result<(), RuntimeError>;
    fn inspect(&self) -> Result<RuntimeStatus, RuntimeError>;
    fn diagnose(&self, settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError>;
}

pub struct UnavailableRuntimeBackend;

impl RuntimeBackend for UnavailableRuntimeBackend {
    fn apply(&self, _settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        Err(RuntimeError::Unavailable(
            "platform runtime has not been installed".into(),
        ))
    }
    fn stop(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn inspect(&self) -> Result<RuntimeStatus, RuntimeError> {
        Ok(RuntimeStatus::Stopped)
    }
    fn diagnose(&self, _settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError> {
        Ok(vec![])
    }
}
