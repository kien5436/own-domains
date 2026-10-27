//! Client error type shared by the Unix and Windows transports.

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon unavailable at {endpoint}: {source}")]
    DaemonUnavailable {
        endpoint: String,
        #[source]
        source: std::io::Error,
    },
    #[error("IPC endpoint is owned by an untrusted server: {message}")]
    UntrustedDaemon { message: String },
    #[error("incompatible protocol: supported {supported}, received {received}")]
    IncompatibleVersion { supported: u16, received: u16 },
    #[error("daemon protocol error: {0:?}")]
    Protocol(own_domains_protocol::ProtocolError),
    #[error("IPC error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid IPC message: {0}")]
    InvalidMessage(#[from] serde_json::Error),
}
