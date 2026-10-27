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

    pub fn default_for_state_dir(state_dir: &Path) -> Result<Self, String> {
        #[cfg(windows)]
        {
            Ok(Self::NamedPipe(default_pipe_name(state_dir)?))
        }
        #[cfg(not(windows))]
        {
            Ok(Self::Unix(state_dir.join("daemon.sock")))
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

/// Windows endpoint derivation (spec: `own-domains-pipe-v1` + user SID +
/// canonical state directory).
///
/// The hash is the first 16 bytes of SHA-256, rendered as 32 lowercase hex
/// characters, over `own-domains-pipe-v1\0 || UTF-8(SID) || \0 ||
/// UTF-16LE(lowercase(canonical_state_directory))`. The canonical directory
/// comes from `GetFullPathNameW` + `PathCchCanonicalizeEx` with any trailing
/// separator removed. This is a naming convention so the desktop, CLI, and
/// daemon agree; the DACL and server-identity checks are the security
/// boundaries. Failures are loud: no fallback name is produced.
#[cfg(windows)]
fn default_pipe_name(state_dir: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HLOCAL, LPARAM, LocalFree},
            Globalization::{LCMAP_LOWERCASE, LCMapStringEx, LOCALE_NAME_INVARIANT},
            Security::{
                Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY,
                TOKEN_USER, TokenUser,
            },
            Storage::FileSystem::GetFullPathNameW,
            System::Threading::{GetCurrentProcess, OpenProcessToken},
            UI::Shell::{PATHCCH_OPTIONS, PathCchCanonicalizeEx},
        },
        core::{PCWSTR, PWSTR},
    };

    // Current user SID as a string.
    let sid = unsafe {
        let mut token = Default::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|error| format!("pipe derivation: OpenProcessToken failed: {error}"))?;
        let sid = (|| {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed == 0 {
                return Err("pipe derivation: token has no user".to_string());
            }
            let mut raw = vec![0u8; needed as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(raw.as_mut_ptr() as *mut _),
                needed,
                &mut needed,
            )
            .map_err(|error| format!("pipe derivation: GetTokenInformation failed: {error}"))?;
            let user = &*(raw.as_ptr() as *const TOKEN_USER);
            let mut wide = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut wide)
                .map_err(|error| format!("pipe derivation: SID string failed: {error}"))?;
            let text = String::from_utf16_lossy(wide.as_wide());
            LocalFree(Some(HLOCAL(wide.as_ptr() as _)));
            Ok(text)
        })();
        let _ = CloseHandle(token);
        sid?
    };

    // Canonical absolute directory, lowercased.
    let wide: Vec<u16> = state_dir.as_os_str().encode_wide().chain([0]).collect();
    let mut full = vec![0u16; 32768];
    let full_len =
        unsafe { GetFullPathNameW(PCWSTR(wide.as_ptr()), Some(full.as_mut_slice()), None) };
    if full_len == 0 || full_len as usize + 1 > full.len() {
        return Err("pipe derivation: cannot absolutize state directory".into());
    }
    full.truncate(full_len as usize + 1);
    let mut canon = vec![0u16; 32768];
    unsafe {
        PathCchCanonicalizeEx(
            canon.as_mut_slice(),
            PCWSTR(full.as_ptr()),
            PATHCCH_OPTIONS(0),
        )
    }
    .map_err(|error| format!("pipe derivation: cannot canonicalize state directory: {error}"))?;
    let canon_len = canon
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("pipe derivation: canonical path is unterminated")?;
    canon.truncate(canon_len + 1);
    let need = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_LOWERCASE,
            &canon,
            None,
            None,
            None,
            LPARAM(0),
        )
    };
    if need == 0 {
        return Err("pipe derivation: cannot lowercase state directory".into());
    }
    let mut lower = vec![0u16; need as usize];
    let mapped = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_LOWERCASE,
            &canon,
            Some(lower.as_mut_slice()),
            None,
            None,
            LPARAM(0),
        )
    };
    if mapped == 0 {
        return Err("pipe derivation: cannot lowercase state directory".into());
    }
    let end = lower
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(lower.len());
    lower.truncate(end);
    while lower.len() > 3 && matches!(lower.last(), Some(0x5C) | Some(0x2F)) {
        lower.pop();
    }
    let lower_le: Vec<u8> = lower.iter().flat_map(|unit| unit.to_le_bytes()).collect();

    let mut digest = Sha256::new();
    digest.update(b"own-domains-pipe-v1\0");
    digest.update(sid.as_bytes());
    digest.update(b"\0");
    digest.update(&lower_le);
    let hash = digest.finalize();
    let hex: String = hash[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("own-domains-{hex}"))
}
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
        let endpoint = Endpoint::default_for_state_dir(&state_dir).unwrap();
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
            assert_eq!(
                endpoint,
                Endpoint::default_for_state_dir(&state_dir).unwrap()
            );
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
