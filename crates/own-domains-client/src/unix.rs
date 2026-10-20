use std::{io::{BufRead, BufReader, Write}, os::unix::net::UnixStream, path::{Path, PathBuf}, time::Duration};
use own_domains_protocol::{Command, CommandResult, EventEnvelope, ProtocolError, RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon unavailable at {path}: {source}", path = path.display())]
    DaemonUnavailable { path: PathBuf, #[source] source: std::io::Error },
    #[error("incompatible protocol: supported {supported}, received {received}")]
    IncompatibleVersion { supported: u16, received: u16 },
    #[error("daemon protocol error: {0:?}")]
    Protocol(ProtocolError),
    #[error("IPC error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid IPC message: {0}")]
    InvalidMessage(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct Client { path: PathBuf, timeout: Duration }

pub struct EventStream { reader: BufReader<UnixStream> }

impl Client {
    pub fn connect(path: &Path) -> Result<Self, ClientError> { Self::with_timeout(path, Duration::from_secs(5)) }

    pub fn with_timeout(path: &Path, timeout: Duration) -> Result<Self, ClientError> {
        let client = Self { path: path.to_path_buf(), timeout };
        let mut stream = client.open()?;
        let result = exchange(&mut stream, Command::Handshake)?;
        match result {
            CommandResult::Handshake { protocol_version: PROTOCOL_VERSION } => Ok(client),
            CommandResult::Handshake { protocol_version } => Err(ClientError::IncompatibleVersion { supported: PROTOCOL_VERSION, received: protocol_version }),
            _ => Err(ClientError::Protocol(ProtocolError::InvalidRequest { message: "daemon did not return handshake".into() })),
        }
    }

    pub fn request(&self, command: Command) -> Result<CommandResult, ClientError> {
        let mut stream = self.open()?;
        let _ = exchange(&mut stream, Command::Handshake)?;
        exchange(&mut stream, command)
    }

    pub fn subscribe(&self, after_sequence: Option<u64>) -> Result<EventStream, ClientError> {
        let mut stream = self.open()?;
        let _ = exchange(&mut stream, Command::Handshake)?;
        let _ = exchange(&mut stream, Command::Subscribe { after_sequence })?;
        Ok(EventStream { reader: BufReader::new(stream) })
    }

    fn open(&self) -> Result<UnixStream, ClientError> {
        let stream = UnixStream::connect(&self.path).map_err(|source| ClientError::DaemonUnavailable { path: self.path.clone(), source })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        Ok(stream)
    }
}

impl EventStream {
    pub fn next(&mut self) -> Result<EventEnvelope, ClientError> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 { return Err(ClientError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "daemon disconnected"))); }
        Ok(serde_json::from_str(&line)?)
    }
}

fn exchange(stream: &mut UnixStream, command: Command) -> Result<CommandResult, ClientError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = RequestEnvelope { protocol_version: PROTOCOL_VERSION, request_id: request_id.clone(), command };
    let frame = serde_json::to_vec(&request)?;
    stream.write_all(&frame)?;
    stream.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(&mut *stream).read_line(&mut line)?;
    if line.is_empty() { return Err(ClientError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "daemon disconnected mid-frame"))); }
    let response: ResponseEnvelope = serde_json::from_str(&line)?;
    if response.request_id != request_id { return Err(ClientError::Protocol(ProtocolError::InvalidRequest { message: "request ID mismatch".into() })); }
    match response.result {
        Ok(result) => Ok(result),
        Err(ProtocolError::IncompatibleVersion { supported, received }) => Err(ClientError::IncompatibleVersion { supported, received }),
        Err(error) => Err(ClientError::Protocol(error)),
    }
}
