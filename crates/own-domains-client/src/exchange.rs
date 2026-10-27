//! Transport-neutral JSON-line handshake, request, and event helpers.
//!
//! Both the Unix socket client and the Windows named-pipe client run this
//! exact sequence: handshake, then one JSON line per command, then newline
//! framing for the subscription stream. Sharing it keeps the protocol
//! byte-identical on both transports.

use crate::error::ClientError;
use own_domains_protocol::{
    Command, CommandResult, EventEnvelope, PROTOCOL_VERSION, ProtocolError, RequestEnvelope,
    ResponseEnvelope, read_frame, write_json_line,
};
use std::io::{Read, Write};

pub fn exchange<S: Read + Write>(
    stream: &mut S,
    command: Command,
) -> Result<CommandResult, ClientError> {
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
