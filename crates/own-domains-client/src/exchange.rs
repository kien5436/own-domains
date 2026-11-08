//! Transport-neutral JSON-line handshake, request, and event helpers.
//!
//! Both the Unix socket client and the Windows named-pipe client run this
//! exact sequence: handshake, then one JSON line per command, then newline
//! framing for the subscription stream. Sharing it keeps the protocol
//! byte-identical on both transports.

use crate::error::ClientError;
use own_domains_protocol::{
    Command, CommandResult, EventEnvelope, MAX_FRAME, PROTOCOL_VERSION, ProtocolError,
    RequestEnvelope, ResponseEnvelope, read_frame, write_json_line,
};
use std::io::{BufRead, BufReader, Read, Write};

pub fn exchange<S: Read + Write>(
    stream: &mut S,
    command: Command,
) -> Result<CommandResult, ClientError> {
    let subscribed = matches!(command, Command::Subscribe { .. });
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        command,
    };
    write_json_line(stream, &request)?;
    // A Subscribe response is followed by the event stream on the same
    // connection, so its frame must be read byte-precise with no prefetch.
    // Every other command gets a fresh connection per request, where
    // buffering is safe and avoids a syscall per byte on large frames.
    //
    // Blocking reads on a socket with SO_RCVTIMEO report deadline expiry
    // as WouldBlock (not TimedOut) on Linux. All client streams carry a
    // read timeout, so WouldBlock here unambiguously means the daemon was
    // too slow — report it as a timeout instead of leaking the raw errno.
    let line = match read_frame_or_buffered(stream, subscribed)? {
        Some(line) => line,
        None => return Err(unexpected_eof()),
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

fn unexpected_eof() -> ClientError {
    std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "daemon disconnected mid-frame",
    )
    .into()
}

fn read_frame_or_buffered<S: Read>(
    stream: &mut S,
    subscribed: bool,
) -> Result<Option<Vec<u8>>, ClientError> {
    let result = if subscribed {
        read_frame(stream)
    } else {
        read_buffered(stream)
    };
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("daemon did not respond in time ({error})"),
        )
        .into()),
        result => result.map_err(Into::into),
    }
}

/// Single-frame read with chunked syscalls. Only for connections that carry
/// no further frames: the buffer may prefetch past the newline.
fn read_buffered<S: Read>(stream: &mut S) -> std::io::Result<Option<Vec<u8>>> {
    let mut reader = BufReader::with_capacity(8192, stream);
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "daemon disconnected mid-frame",
            ));
        }
        let end = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + 1)
            .unwrap_or(chunk.len());
        if line.len() + end > MAX_FRAME + 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response exceeds 1 MiB",
            ));
        }
        line.extend_from_slice(&chunk[..end]);
        reader.consume(end);
        if line.ends_with(b"\n") {
            line.pop();
            return Ok(Some(line));
        }
    }
}

pub fn handshake<S: Read + Write>(stream: &mut S) -> Result<(), ClientError> {
    match exchange(stream, Command::Handshake)? {
        CommandResult::Handshake {
            protocol_version: PROTOCOL_VERSION,
        } => Ok(()),
        CommandResult::Handshake { protocol_version } => Err(ClientError::IncompatibleVersion {
            supported: PROTOCOL_VERSION,
            received: protocol_version,
        }),
        _ => Err(ClientError::Protocol(ProtocolError::InvalidRequest {
            message: "daemon did not return handshake".into(),
        })),
    }
}

pub fn parse_event(frame: &[u8]) -> Result<EventEnvelope, ClientError> {
    let event: EventEnvelope = serde_json::from_slice(frame)?;
    if event.protocol_version != PROTOCOL_VERSION {
        return Err(ClientError::IncompatibleVersion {
            supported: PROTOCOL_VERSION,
            received: event.protocol_version,
        });
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream whose reads always report WouldBlock, as Linux does when a
    /// `SO_RCVTIMEO` read deadline expires. Writes succeed into a sink.
    struct WouldBlockStream {
        written: Vec<u8>,
    }

    impl Read for WouldBlockStream {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Resource temporarily unavailable (os error 11)",
            ))
        }
    }

    impl Write for WouldBlockStream {
        fn write(&mut self, chunk: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(chunk);
            Ok(chunk.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn would_block_read_reports_a_timeout_not_a_raw_os_error() {
        let mut stream = WouldBlockStream {
            written: Vec::new(),
        };
        let error = exchange(&mut stream, Command::Snapshot).unwrap_err();
        let message = error.to_string();
        assert!(
            !message.contains("IPC error: Resource temporarily unavailable"),
            "leaked raw WouldBlock as the whole error: {message}"
        );
        assert!(
            message.contains("did not respond in time"),
            "expected a timeout error, got: {message}"
        );
    }
}
