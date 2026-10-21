use own_domains_core::{ApplyResult, Diagnostic, MappingInput, MappingPatch, Settings};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    pub protocol_version: u16,
    pub request_id: String,
    pub command: Command,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub protocol_version: u16,
    pub request_id: String,
    pub result: Result<CommandResult, ProtocolError>,
}

impl ResponseEnvelope {
    pub fn ok(request_id: impl Into<String>, result: CommandResult) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            result: Ok(result),
        }
    }

    pub fn error(request_id: impl Into<String>, error: ProtocolError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            result: Err(error),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub protocol_version: u16,
    pub sequence: u64,
    pub event: Event,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum Command {
    Handshake,
    Snapshot,
    Subscribe {
        after_sequence: Option<u64>,
    },
    Start,
    Stop,
    List,
    Create {
        input: MappingInput,
        expected_revision: u64,
    },
    Update {
        id: String,
        patch: MappingPatch,
        expected_revision: u64,
    },
    Remove {
        id: String,
        expected_revision: u64,
    },
    UpdateNetwork {
        lan_address: String,
        upstream_dns: Vec<String>,
        expected_revision: u64,
    },
    Doctor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum CommandResult {
    Handshake { protocol_version: u16 },
    Snapshot(StateSnapshot),
    Settings(Settings),
    Mutation(MutationResult),
    Diagnostics(Vec<Diagnostic>),
    Runtime(RuntimeStatus),
    Apply(ApplyResult),
    Subscribed,
    Acknowledged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResult {
    pub settings: Settings,
    pub apply: Option<ApplyResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum ProtocolError {
    IncompatibleVersion { supported: u16, received: u16 },
    Validation { message: String },
    RevisionConflict { expected: u64, actual: u64 },
    NotFound { message: String },
    DuplicateHostname { hostname: String },
    RuntimeUnavailable { message: String },
    RuntimeFailure { message: String },
    Storage { message: String },
    AlreadyRunning { path: String },
    InvalidRequest { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeStatus {
    Stopped,
    Starting,
    Running,
    Degraded,
    Stopping,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub settings: Settings,
    pub runtime_status: RuntimeStatus,
    pub desired_revision: u64,
    pub applied_revision: u64,
    pub diagnostics: Vec<Diagnostic>,
    pub latest_event_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum Event {
    SettingsChanged { revision: u64 },
    RuntimeChanged { status: RuntimeStatus },
    DiagnosticsChanged { diagnostics: Vec<Diagnostic> },
    ResyncRequired { snapshot: StateSnapshot },
    Snapshot(StateSnapshot),
}

pub fn validate_version(received: u16) -> Result<(), ProtocolError> {
    if received == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::IncompatibleVersion {
            supported: PROTOCOL_VERSION,
            received,
        })
    }
}
