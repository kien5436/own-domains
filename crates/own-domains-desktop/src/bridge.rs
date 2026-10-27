use own_domains_client::{Client, ClientError, Endpoint};
use own_domains_protocol::{Command, CommandResult, ProtocolError};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum BridgeError {
    Unavailable { message: String },
    Incompatible { supported: u16, received: u16 },
    Protocol { error: ProtocolError },
    Transport { message: String },
    InvalidMessage { message: String },
}
impl From<ClientError> for BridgeError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::DaemonUnavailable { .. } => Self::Unavailable {
                message: error.to_string(),
            },
            ClientError::IncompatibleVersion {
                supported,
                received,
            } => Self::Incompatible {
                supported,
                received,
            },
            ClientError::Protocol(error) => Self::Protocol { error },
            ClientError::InvalidMessage(error) => Self::InvalidMessage {
                message: error.to_string(),
            },
            ClientError::Io(error) => Self::Transport {
                message: error.to_string(),
            },
        }
    }
}

#[derive(Clone)]
pub struct DesktopPaths {
    pub state_dir: PathBuf,
    pub endpoint: Endpoint,
}
impl DesktopPaths {
    pub fn new(
        state_dir: Option<&Path>,
        socket: Option<&Path>,
        pipe: Option<&str>,
    ) -> Result<Self, BridgeError> {
        let state_dir = state_dir
            .filter(|path| path.is_absolute())
            .ok_or_else(|| BridgeError::InvalidMessage {
                message:
                    "Provide an absolute --state-dir pointing to your existing settings directory"
                        .into(),
            })?
            .to_path_buf();
        if socket.is_some() && pipe.is_some() {
            return Err(BridgeError::InvalidMessage {
                message: "use either --socket or --pipe".into(),
            });
        }
        let endpoint = match (socket, pipe) {
            (_, Some(name)) => Endpoint::named_pipe(name.into())
                .map_err(|message| BridgeError::InvalidMessage { message })?,
            (Some(path), None) => {
                if !path.is_absolute() {
                    return Err(BridgeError::InvalidMessage {
                        message: "--socket must be absolute".into(),
                    });
                }
                Endpoint::unix(path.to_path_buf())
            }
            (None, None) => Endpoint::default_for_state_dir(&state_dir),
        };
        Ok(Self {
            state_dir,
            endpoint,
        })
    }
}

#[derive(Clone)]
pub struct DesktopBridge {
    pub(crate) client: Client,
}
impl DesktopBridge {
    pub fn connect(endpoint: &Endpoint) -> Result<Self, BridgeError> {
        Ok(Self {
            client: Client::connect(endpoint)?,
        })
    }
    pub fn request(&self, command: Command) -> Result<CommandResult, BridgeError> {
        if matches!(command, Command::Handshake | Command::Subscribe { .. }) {
            return Err(BridgeError::InvalidMessage {
                message: "The desktop bridge owns handshake and subscription commands".into(),
            });
        }
        self.client.request(command).map_err(Into::into)
    }
}
