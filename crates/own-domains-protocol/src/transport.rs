use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Maximum size of a single newline-delimited IPC frame in bytes.
pub const MAX_FRAME: usize = 1024 * 1024;

/// Platform IPC endpoint the daemon listens on and clients connect to.
///
/// Unix sockets are the only listening transport today. The Windows named-pipe
/// variant is constructed on Windows only; the real pipe server/client lands next.
/// Shared command code never branches on the operating system.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Unix(PathBuf),
    NamedPipe(String),
}

impl Endpoint {
    pub fn unix(path: PathBuf) -> Self {
        Self::Unix(path)
    }

    pub fn named_pipe(name: String) -> Result<Self, String> {
        #[cfg(windows)]
        {
            Ok(Self::NamedPipe(name))
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err("named-pipe IPC endpoints are only supported on Windows".into())
        }
    }

    pub fn default_for_state_dir(state_dir: &Path) -> Self {
        #[cfg(windows)]
        {
            Self::NamedPipe(default_pipe_name(state_dir))
        }
        #[cfg(not(windows))]
        {
            Self::Unix(state_dir.join("daemon.sock"))
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::Unix(path) => path.display().to_string(),
            Self::NamedPipe(name) => format!(r"\\.\pipe\{name}"),
        }
    }

    /// Daemon launch argument reproducing this endpoint, if any.
    pub fn launch_argument(&self) -> Option<(&'static str, OsString)> {
        match self {
            Self::Unix(path) => Some(("--socket", path.as_os_str().to_os_string())),
            Self::NamedPipe(name) => Some(("--pipe", OsString::from(name))),
        }
    }

    /// Whether the endpoint is absent, as opposed to failing for another reason.
    pub fn is_absent_error(&self, error: &io::Error) -> bool {
        match self {
            Self::Unix(_) => matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ),
            // A missing pipe surfaces as NotFound. A busy pipe means a live
            // server still owns the name, so it is never absence.
            Self::NamedPipe(_) => matches!(error.kind(), io::ErrorKind::NotFound),
        }
    }

    /// Whether an endpoint file currently exists. Used to wait for daemon shutdown.
    pub fn is_present(&self) -> bool {
        match self {
            Self::Unix(path) => std::fs::symlink_metadata(path).is_ok(),
            // Real presence check (CreateFileW probe) lands with the pipe client.
            Self::NamedPipe(_) => false,
        }
    }
}

/// Interim Windows pipe-name derivation (TODO: spec SID + canonical-path hash).
///
/// Produces a stable `own-domains-<16 hex>` name from the state directory so the
/// desktop, CLI, and daemon agree on one endpoint before the full
/// `own-domains-pipe-v1 + SID + canonical path` derivation lands with the
/// `windows` crate.
#[cfg(windows)]
fn default_pipe_name(state_dir: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    state_dir.as_os_str().len().hash(&mut hasher);
    state_dir.as_os_str().hash(&mut hasher);
    format!("own-domains-{:016x}", hasher.finish())
}

/// Read one newline-terminated frame. Returns `None` on clean EOF before any byte.
pub fn read_frame<R: Read>(stream: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    let mut byte = [0];
    loop {
        match stream.read(&mut byte)? {
            0 if frame.is_empty() => return Ok(None),
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "IPC frame is missing newline",
                ));
            }
            _ if byte[0] == b'\n' => return Ok(Some(frame)),
            _ => {
                frame.push(byte[0]);
                if frame.len() > MAX_FRAME {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "IPC frame exceeds 1 MiB",
                    ));
                }
            }
        }
    }
}

pub fn write_json_line<W: Write>(stream: &mut W, value: &impl serde::Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_defaults_to_the_state_directory_socket() {
        let state_dir = std::env::temp_dir().join("od-endpoint-state");
        let endpoint = Endpoint::default_for_state_dir(&state_dir);
        #[cfg(not(windows))]
        {
            let expected = state_dir.join("daemon.sock");
            assert_eq!(endpoint, Endpoint::Unix(expected.clone()));
            assert!(endpoint.display_name().ends_with("daemon.sock"));
            assert_eq!(
                endpoint.launch_argument(),
                Some(("--socket", expected.as_os_str().to_os_string()))
            );
        }
        #[cfg(windows)]
        {
            let Endpoint::NamedPipe(name) = &endpoint else {
                panic!("expected NamedPipe default on Windows, got {endpoint:?}");
            };
            assert!(name.starts_with("own-domains-"));
            assert!(endpoint.display_name().starts_with(r"\\.\pipe\"));
            assert_eq!(
                endpoint.launch_argument(),
                Some(("--pipe", OsString::from(name)))
            );
        }
    }

    #[test]
    fn endpoint_classifies_absence_without_claiming_other_errors() {
        let endpoint = Endpoint::unix(PathBuf::from("/tmp/state/daemon.sock"));
        assert!(endpoint.is_absent_error(&io::Error::new(io::ErrorKind::NotFound, "gone")));
        assert!(
            endpoint.is_absent_error(&io::Error::new(io::ErrorKind::ConnectionRefused, "refused"))
        );
        assert!(
            !endpoint.is_absent_error(&io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
        );
        assert!(!endpoint.is_present());
    }

    #[test]
    fn named_pipe_is_rejected_off_windows() {
        #[cfg(not(target_os = "windows"))]
        assert!(Endpoint::named_pipe("own-domains-test".into()).is_err());
        #[cfg(target_os = "windows")]
        assert!(Endpoint::named_pipe("own-domains-test".into()).is_ok());
    }

    #[test]
    fn frames_round_trip_and_reject_oversized_input() {
        let mut buffer = Vec::new();
        write_json_line(&mut buffer, &serde_json::json!({"hello": "world"})).unwrap();
        let frame = read_frame(&mut &buffer[..]).unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&frame).unwrap(),
            serde_json::json!({"hello": "world"})
        );
        assert!(read_frame(&mut &b""[..]).unwrap().is_none());
        let mut oversized = vec![b'a'; MAX_FRAME + 1];
        oversized.push(b'\n');
        assert!(read_frame(&mut &oversized[..]).is_err());
    }
}
