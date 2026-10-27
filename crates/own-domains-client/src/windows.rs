//! Windows named-pipe client.
//!
//! The pipe is byte mode, so the shared JSON-line handshake and framing in
//! `super::exchange` run unchanged. Every connection verifies the server
//! before sending a byte: PID ownership, image path, user SID, and session,
//! while the open process handle pins the PID against recycling for the life
//! of the connection. (Authenticode publisher pinning lands with the signed
//! packaging gate; until then the image/SID/session checks are the boundary.)

use crate::{
    error::ClientError,
    exchange::{exchange, handshake, parse_event},
};
use own_domains_protocol::{Command, CommandResult, Endpoint, EventEnvelope, pipe::PipeStream};
use std::{
    io::{self, Read, Write},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, FILETIME, HANDLE, HLOCAL, LocalFree,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING},
        System::{
            Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW},
            RemoteDesktop::ProcessIdToSessionId,
            Threading::{
                GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
                PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};

fn unavailable(endpoint: &Endpoint, source: io::Error) -> ClientError {
    ClientError::DaemonUnavailable {
        endpoint: endpoint.display_name(),
        source,
    }
}

fn unsupported(endpoint: &Endpoint) -> ClientError {
    unavailable(
        endpoint,
        io::Error::new(
            io::ErrorKind::Unsupported,
            "unix sockets need the unix client",
        ),
    )
}

fn untrusted(message: String) -> ClientError {
    ClientError::UntrustedDaemon { message }
}

/// Expected daemon image: explicit override first, else the daemon installed
/// next to this binary. Mirrors the launch resolution in the desktop shell
/// (`OWN_DOMAINS_DAEMON` or `current_exe` sibling) so identity always matches
/// what this frontend would launch.
fn expected_daemon_image() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("OWN_DOMAINS_DAEMON") {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path);
        }
        return Err("OWN_DOMAINS_DAEMON must be an absolute path".into());
    }
    std::env::current_exe()
        .map(|exe| {
            exe.with_file_name(format!(
                "own-domains-daemon{}",
                std::env::consts::EXE_SUFFIX
            ))
        })
        .map_err(|error| format!("cannot locate daemon executable: {error}"))
}

fn canonical(path: &std::path::Path) -> Result<String, String> {
    use windows::Win32::Storage::FileSystem::GetFullPathNameW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut full = vec![0u16; 32768];
    let len = unsafe { GetFullPathNameW(PCWSTR(wide.as_ptr()), Some(full.as_mut_slice()), None) };
    if len == 0 || len as usize + 1 > full.len() {
        return Err(format!("cannot absolutize {}", path.display()));
    }
    let text = String::from_utf16_lossy(&full[..len as usize]);
    Ok(strip_long_prefix(&text).to_lowercase())
}

fn strip_long_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

/// Open server process handle. Dropping closes it; while alive the PID cannot
/// be recycled into a different process underneath this connection.
struct ServerGuard(HANDLE);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn connect_pipe(endpoint: &Endpoint, timeout: Duration) -> Result<PipeStream, ClientError> {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    let wide: Vec<u16> = endpoint.display_name().encode_utf16().chain([0]).collect();
    let deadline = Instant::now() + timeout;
    loop {
        match unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        } {
            Ok(handle) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                return PipeStream::new(handle, remaining).map_err(|source| {
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    unavailable(endpoint, source)
                });
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_BUSY.0) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(unavailable(
                        endpoint,
                        io::Error::new(io::ErrorKind::TimedOut, "pipe busy"),
                    ));
                }
                let millis = remaining.as_millis().min(u32::MAX as u128) as u32;
                if !unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), millis).as_bool() } {
                    let error = windows::core::Error::from_thread();
                    if error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) {
                        return Err(unavailable(
                            endpoint,
                            io::Error::new(io::ErrorKind::NotFound, "pipe server is gone"),
                        ));
                    }
                    return Err(unavailable(endpoint, error.into()));
                }
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => {
                return Err(unavailable(
                    endpoint,
                    io::Error::new(io::ErrorKind::NotFound, "no pipe server listening"),
                ));
            }
            Err(error) => return Err(unavailable(endpoint, error.into())),
        }
    }
}

fn verify_server(pipe: HANDLE, timeout: Duration) -> Result<ServerGuard, ClientError> {
    let _ = timeout;
    let mut pid = 0u32;
    unsafe { GetNamedPipeServerProcessId(pipe, &mut pid) }
        .map_err(|error| untrusted(format!("cannot identify pipe server: {error}")))?;
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| untrusted(format!("cannot open pipe server {pid}: {error}")))?;

    let image = unsafe {
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .map_err(|error| {
            let _ = CloseHandle(process);
            untrusted(format!("cannot read pipe server image: {error}"))
        })?;
        String::from_utf16_lossy(&buf[..len as usize])
    };
    let expected = expected_daemon_image()
        .and_then(|path| canonical(&path))
        .map_err(|message| {
            unsafe {
                let _ = CloseHandle(process);
            }
            untrusted(message)
        })?;
    if strip_long_prefix(&image).to_lowercase() != expected {
        unsafe {
            let _ = CloseHandle(process);
        }
        return Err(untrusted(format!(
            "pipe server image is not the expected daemon: {image}"
        )));
    }

    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut user = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }.map_err(
        |error| {
            unsafe {
                let _ = CloseHandle(process);
            }
            untrusted(format!("pipe server vanished during verification: {error}"))
        },
    )?;
    let _ = creation;

    let server_sid = unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).map_err(|error| {
            let _ = CloseHandle(process);
            untrusted(format!("cannot open pipe server token: {error}"))
        })?;
        let sid = (|| {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed == 0 {
                return Err("pipe server token has no user".to_string());
            }
            let mut raw = vec![0u8; needed as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(raw.as_mut_ptr() as *mut _),
                needed,
                &mut needed,
            )
            .map_err(|error| format!("cannot read pipe server user: {error}"))?;
            let user = &*(raw.as_ptr() as *const TOKEN_USER);
            let mut wide = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut wide)
                .map_err(|error| format!("cannot read pipe server user: {error}"))?;
            let text = String::from_utf16_lossy(wide.as_wide());
            LocalFree(Some(HLOCAL(wide.as_ptr() as _)));
            Ok(text)
        })();
        let _ = CloseHandle(token);
        sid.map_err(untrusted)?
    };
    let own_sid = own_domains_protocol::pipe::current_user_sid()
        .map_err(|message| untrusted(format!("cannot read current user: {message}")))?;
    if server_sid != own_sid {
        unsafe {
            let _ = CloseHandle(process);
        }
        return Err(untrusted("pipe server runs as a different user".into()));
    }

    let (mut server_session, mut own_session) = (0u32, 0u32);
    unsafe {
        ProcessIdToSessionId(pid, &mut server_session).and(ProcessIdToSessionId(
            GetCurrentProcessId(),
            &mut own_session,
        ))
    }
    .map_err(|error| {
        unsafe {
            let _ = CloseHandle(process);
        }
        untrusted(format!("cannot read session: {error}"))
    })?;
    if server_session != own_session {
        unsafe {
            let _ = CloseHandle(process);
        }
        return Err(untrusted("pipe server is in a different session".into()));
    }

    Ok(ServerGuard(process))
}

#[derive(Clone)]
pub struct Client {
    endpoint: Endpoint,
    timeout: Duration,
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
        let (mut stream, _server) = client.open_verified(timeout)?;
        handshake(&mut stream)?;
        Ok(client)
    }

    pub fn request(&self, command: Command) -> Result<CommandResult, ClientError> {
        let (mut stream, _server) = self.open_verified(self.timeout)?;
        handshake(&mut stream)?;
        exchange(&mut stream, command)
    }

    pub fn subscribe(&self, after_sequence: Option<u64>) -> Result<EventStream, ClientError> {
        let (mut stream, _server) = self.open_verified(self.timeout)?;
        handshake(&mut stream)?;
        exchange(&mut stream, Command::Subscribe { after_sequence })?;
        let cancel = Arc::new(AtomicBool::new(false));
        stream.set_cancel(cancel.clone());
        Ok(EventStream {
            pipe: stream,
            pending: Vec::new(),
            cancel,
        })
    }

    fn open_verified(&self, timeout: Duration) -> Result<(PipeStream, ServerGuard), ClientError> {
        let Endpoint::NamedPipe(_) = &self.endpoint else {
            return Err(unsupported(&self.endpoint));
        };
        let stream = connect_pipe(&self.endpoint, timeout)?;
        // On failure the stream drops here and closes the pipe; on success the
        // guard keeps the server PID pinned for the life of this connection.
        let server = verify_server(stream.handle(), timeout)?;
        Ok((stream, server))
    }
}

pub struct EventStream {
    pipe: PipeStream,
    pending: Vec<u8>,
    cancel: Arc<AtomicBool>,
}

pub struct EventStreamCancellation {
    cancel: Arc<AtomicBool>,
}

impl EventStreamCancellation {
    pub fn cancel(&self) -> Result<(), ClientError> {
        self.cancel.store(true, Ordering::Release);
        Ok(())
    }
}

impl EventStream {
    pub fn cancellation_handle(&self) -> Result<EventStreamCancellation, ClientError> {
        Ok(EventStreamCancellation {
            cancel: self.cancel.clone(),
        })
    }

    pub fn recv(&mut self) -> Result<EventEnvelope, ClientError> {
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let frame: Vec<u8> = self.pending.drain(..=end).collect();
                return parse_event(&frame[..frame.len() - 1]);
            }
            match self.pipe.read(&mut chunk) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "daemon disconnected mid-stream",
                    )
                    .into());
                }
                Ok(read) => {
                    self.pending.extend_from_slice(&chunk[..read]);
                    if self.pending.len() > 1024 * 1024 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "event frame exceeds 1 MiB",
                        )
                        .into());
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Read for EventStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.pipe.read(buf)
    }
}

impl Write for EventStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pipe.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.pipe.flush()
    }
}
