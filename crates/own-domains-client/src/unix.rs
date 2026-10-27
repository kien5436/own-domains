use own_domains_protocol::{
    Command, CommandResult, Endpoint, EventEnvelope, PROTOCOL_VERSION, ProtocolError,
    RequestEnvelope, ResponseEnvelope, read_frame, write_json_line,
};
use std::{
    io::{BufReader, ErrorKind, Read},
    os::unix::net::UnixStream,
    time::Duration,
};

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
    Protocol(ProtocolError),
    #[error("IPC error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid IPC message: {0}")]
    InvalidMessage(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct Client {
    endpoint: Endpoint,
    timeout: Duration,
}

pub struct EventStream {
    reader: BufReader<UnixStream>,
    pending: Vec<u8>,
}

impl Client {
    pub fn connect(endpoint: &Endpoint) -> Result<Self, ClientError> {
        Self::with_timeout(endpoint, Duration::from_secs(5))
    }

    pub fn with_timeout(endpoint: &Endpoint, timeout: Duration) -> Result<Self, ClientError> {
        let client = Self {
            endpoint: endpoint.clone(),
            timeout,
        };
        let mut stream = client.open()?;
        let result = exchange(&mut stream, Command::Handshake)?;
        match result {
            CommandResult::Handshake {
                protocol_version: PROTOCOL_VERSION,
            } => Ok(client),
            CommandResult::Handshake { protocol_version } => {
                Err(ClientError::IncompatibleVersion {
                    supported: PROTOCOL_VERSION,
                    received: protocol_version,
                })
            }
            _ => Err(ClientError::Protocol(ProtocolError::InvalidRequest {
                message: "daemon did not return handshake".into(),
            })),
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
        Ok(EventStream {
            reader: BufReader::new(stream),
            pending: Vec::new(),
        })
    }

    fn open(&self) -> Result<UnixStream, ClientError> {
        let Endpoint::Unix(path) = &self.endpoint else {
            return Err(ClientError::DaemonUnavailable {
                endpoint: self.endpoint.display_name(),
                source: std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "named-pipe endpoints need the Windows client",
                ),
            });
        };
        let stream =
            UnixStream::connect(path).map_err(|source| ClientError::DaemonUnavailable {
                endpoint: self.endpoint.display_name(),
                source,
            })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        Ok(stream)
    }
}

pub struct EventStreamCancellation {
    stream: UnixStream,
}
impl EventStreamCancellation {
    pub fn cancel(&self) -> Result<(), ClientError> {
        match self.stream.shutdown(std::net::Shutdown::Both) {
            // Stopping the daemon closes the peer before the frontend cleans up
            // its subscription. That cleanup is already complete in that case.
            Err(error) if error.kind() == ErrorKind::NotConnected => Ok(()),
            result => result.map_err(Into::into),
        }
    }
}
impl EventStream {
    pub fn cancellation_handle(&self) -> Result<EventStreamCancellation, ClientError> {
        Ok(EventStreamCancellation {
            stream: self.reader.get_ref().try_clone()?,
        })
    }
    pub fn recv(&mut self) -> Result<EventEnvelope, ClientError> {
        let mut byte = [0];
        loop {
            match self.reader.read(&mut byte) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "daemon disconnected mid-stream",
                    )
                    .into());
                }
                Ok(_) if byte[0] == b'\n' => {
                    let frame = std::mem::take(&mut self.pending);
                    let event: EventEnvelope = serde_json::from_slice(&frame)?;
                    if event.protocol_version != PROTOCOL_VERSION {
                        return Err(ClientError::IncompatibleVersion {
                            supported: PROTOCOL_VERSION,
                            received: event.protocol_version,
                        });
                    }
                    return Ok(event);
                }
                Ok(_) => {
                    self.pending.push(byte[0]);
                    if self.pending.len() > 1024 * 1024 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "event frame exceeds 1 MiB",
                        )
                        .into());
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn exchange(stream: &mut UnixStream, command: Command) -> Result<CommandResult, ClientError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        command,
    };
    write_json_line(stream, &request)?;
    // Do not prefetch bytes belonging to the subscription's first event.
    let line = match read_frame(stream)? {
        Some(line) => line,
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "daemon disconnected mid-frame",
            )
            .into());
        }
    };
    let response: ResponseEnvelope = serde_json::from_slice(&line)?;
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ClientError::IncompatibleVersion {
            supported: PROTOCOL_VERSION,
            received: response.protocol_version,
        });
    }
    if response.request_id != request_id {
        return Err(ClientError::Protocol(ProtocolError::InvalidRequest {
            message: "request ID mismatch".into(),
        }));
    }
    match response.result {
        Ok(result) => Ok(result),
        Err(ProtocolError::IncompatibleVersion {
            supported,
            received,
        }) => Err(ClientError::IncompatibleVersion {
            supported,
            received,
        }),
        Err(error) => Err(ClientError::Protocol(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::EventStreamCancellation;
    use std::os::unix::net::UnixStream;

    #[test]
    fn cancellation_tolerates_an_already_closed_connection() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let cancel = EventStreamCancellation { stream };

        drop(peer);
        cancel.cancel().unwrap();
        assert!(cancel.cancel().is_ok());
    }
}
