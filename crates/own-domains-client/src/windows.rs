//! Windows client stub. The real named-pipe transport lands next; this exists so
//! the workspace compiles on Windows and every failure is a typed
//! `DaemonUnavailable` rather than a missing symbol.

use own_domains_protocol::{Command, CommandResult, Endpoint, EventEnvelope};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon unavailable at {endpoint}: {source}")]
    DaemonUnavailable {
        endpoint: String,
        #[source]
        source: std::io::Error,
    },
    #[error("incompatible protocol: supported {supported}, received {received}")]
    IncompatibleVersion { supported: u16, received: u16 },
    #[error("daemon protocol error: {0:?}")]
    Protocol(own_domains_protocol::ProtocolError),
    #[error("IPC error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid IPC message: {0}")]
    InvalidMessage(#[from] serde_json::Error),
}

fn unavailable(endpoint: &Endpoint) -> ClientError {
    ClientError::DaemonUnavailable {
        endpoint: endpoint.display_name(),
        source: std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "named-pipe IPC is not implemented yet",
        ),
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct Client {
    endpoint: Endpoint,
    timeout: Duration,
}

impl Client {
    pub fn connect(endpoint: &Endpoint) -> Result<Self, ClientError> {
        Self::with_timeout(endpoint, Duration::from_secs(5))
    }

    pub fn with_timeout(endpoint: &Endpoint, timeout: Duration) -> Result<Self, ClientError> {
        let _ = timeout;
        Err(unavailable(endpoint))
    }

    pub fn request(&self, _command: Command) -> Result<CommandResult, ClientError> {
        Err(unavailable(&self.endpoint))
    }

    pub fn subscribe(&self, _after_sequence: Option<u64>) -> Result<EventStream, ClientError> {
        Err(unavailable(&self.endpoint))
    }
}

pub struct EventStream {
    _priv: (),
}

impl EventStream {
    pub fn cancellation_handle(&self) -> Result<EventStreamCancellation, ClientError> {
        Err(ClientError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "named-pipe IPC is not implemented yet",
        )))
    }

    pub fn recv(&mut self) -> Result<EventEnvelope, ClientError> {
        Err(ClientError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "named-pipe IPC is not implemented yet",
        )))
    }
}

pub struct EventStreamCancellation {
    _priv: (),
}

impl EventStreamCancellation {
    pub fn cancel(&self) -> Result<(), ClientError> {
        Ok(())
    }
}
