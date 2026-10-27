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
    let line = if subscribed {
        match read_frame(stream)? {
            Some(line) => line,
            None => return Err(unexpected_eof()),
        }
    } else {
        match read_buffered(stream)? {
            Some(line) => line,
            None => return Err(unexpected_eof()),
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

fn unexpected_eof() -> ClientError {
    std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "daemon disconnected mid-frame",
    )
    .into()
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
