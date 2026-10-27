use crate::{
    error::ClientError,
    exchange::{exchange, handshake, parse_event},
};
use own_domains_protocol::{Command, CommandResult, Endpoint, EventEnvelope};
use std::{
    io::{BufReader, ErrorKind, Read},
    os::unix::net::UnixStream,
    time::Duration,
};

/// Platform launch tweaks for the daemon child process.
pub(crate) fn configure_daemon_command(_command: &mut std::process::Command) {}

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
        handshake(&mut stream)?;
        Ok(client)
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
                    return parse_event(&frame);
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
