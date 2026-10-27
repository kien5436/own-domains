use crate::{DaemonError, DaemonState};
use own_domains_protocol::{Command, Endpoint, RequestEnvelope, read_frame, write_json_line};
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    sync::Arc,
};
#[cfg(windows)]
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE};

#[cfg(unix)]
pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
    state: Arc<DaemonState>,
    shutdown: Arc<AtomicBool>,
}

#[cfg(unix)]
impl IpcServer {
    pub fn bind(endpoint: &Endpoint, state: Arc<DaemonState>) -> Result<Self, DaemonError> {
        let Endpoint::Unix(path) = endpoint else {
            return Err(DaemonError::Io {
                operation: "bind IPC endpoint",
                path: PathBuf::from(endpoint.display_name()),
                source: io::Error::new(
                    io::ErrorKind::Unsupported,
                    "named-pipe endpoints need the Windows server",
                ),
            });
        };
        let parent = path
            .parent()
            .ok_or_else(|| DaemonError::InvalidSocketPath {
                path: path.to_path_buf(),
            })?;
        fs::create_dir_all(parent)
            .map_err(|source| io_error("create IPC directory", parent, source))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|source| io_error("secure IPC directory", parent, source))?;
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() || UnixStream::connect(path).is_ok() {
                return Err(DaemonError::InvalidSocketPath {
                    path: path.to_path_buf(),
                });
            }
            fs::remove_file(path)
                .map_err(|source| io_error("remove stale IPC socket", path, source))?;
        }
        let listener =
            UnixListener::bind(path).map_err(|source| io_error("bind IPC socket", path, source))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|source| io_error("secure IPC socket", path, source))?;
        let metadata =
            fs::metadata(path).map_err(|source| io_error("inspect IPC socket", path, source))?;
        listener
            .set_nonblocking(true)
            .map_err(|source| io_error("configure IPC socket", path, source))?;
        Ok(Self {
            listener,
            path: path.to_path_buf(),
            identity: (metadata.dev(), metadata.ino()),
            state,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn run(self) -> Result<(), DaemonError> {
        while !self.shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(false).is_err() {
                        continue;
                    }
                    let state = self.state.clone();
                    let shutdown = self.shutdown.clone();
                    thread::spawn(move || handle_client(stream, state, shutdown));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                    ) =>
                {
                    continue;
                }
                Err(source) => return Err(io_error("accept IPC client", &self.path, source)),
            }
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub fn request_shutdown(endpoint: &Endpoint) {
        let Endpoint::Unix(path) = endpoint else {
            return;
        };
        if let Ok(mut stream) = UnixStream::connect(path) {
            let request = RequestEnvelope {
                protocol_version: own_domains_protocol::PROTOCOL_VERSION,
                request_id: "shutdown".into(),
                command: Command::Stop,
            };
            let _ = write_json_line(&mut stream, &request);
        }
    }
}

#[cfg(unix)]
impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::metadata(&self.path)
            && (metadata.dev(), metadata.ino()) == self.identity
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn handle_client<S: Read + Write>(
    mut stream: S,
    state: Arc<DaemonState>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        let frame = match read_frame(&mut stream) {
            Ok(Some(frame)) => frame,
            _ => return,
        };
        let request: RequestEnvelope = match serde_json::from_slice(&frame) {
            Ok(request) => request,
            Err(_) => return,
        };
        let subscribing = match request.command {
            Command::Subscribe { after_sequence } => Some(after_sequence),
            _ => None,
        };
        let stopping = matches!(request.command, Command::Stop);
        let response = state.handle(request);
        if stopping && response.result.is_ok() {
            shutdown.store(true, Ordering::Release);
        }
        if write_json_line(&mut stream, &response).is_err() {
            return;
        }
        if let Some(after_sequence) = subscribing {
            let subscription = state.events().subscribe(after_sequence, || {
                state.snapshot().unwrap_or_else(|_| empty_snapshot())
            });
            while let Ok(event) = subscription.recv() {
                if write_json_line(&mut stream, &event).is_err() {
                    break;
                }
            }
            return;
        }
    }
}

fn empty_snapshot() -> own_domains_protocol::StateSnapshot {
    own_domains_protocol::StateSnapshot {
        settings: Default::default(),
        runtime_status: own_domains_protocol::RuntimeStatus::Error,
        desired_revision: 0,
        applied_revision: 0,
        diagnostics: vec![],
        latest_event_sequence: 0,
    }
}

#[cfg(unix)]
fn io_error(operation: &'static str, path: &Path, source: io::Error) -> DaemonError {
    DaemonError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}
#[cfg(windows)]
pub struct IpcServer {
    full_name: String,
    sid: String,
    first: HANDLE,
    shutdown: Arc<AtomicBool>,
    state: Arc<DaemonState>,
}

#[cfg(windows)]
impl IpcServer {
    pub fn bind(endpoint: &Endpoint, state: Arc<DaemonState>) -> Result<Self, DaemonError> {
        let Endpoint::NamedPipe(_) = endpoint else {
            return Err(DaemonError::Io {
                operation: "bind IPC endpoint",
                path: PathBuf::from(endpoint.display_name()),
                source: io::Error::new(
                    io::ErrorKind::Unsupported,
                    "unix sockets need the unix server",
                ),
            });
        };
        let full_name = endpoint.display_name();
        let sid =
            own_domains_protocol::pipe::current_user_sid().map_err(|message| DaemonError::Io {
                operation: "bind IPC endpoint",
                path: PathBuf::from(&full_name),
                source: io::Error::new(io::ErrorKind::PermissionDenied, message),
            })?;
        let first = Self::create_instance(&full_name, &sid, true)?;
        Ok(Self {
            full_name,
            sid,
            first,
            shutdown: Arc::new(AtomicBool::new(false)),
            state,
        })
    }

    /// Create one pipe instance with the explicit owner-only DACL.
    ///
    /// Every instance carries the protected DACL: the default named-pipe DACL
    /// grants `Everyone` read access and must never be used. `first` sets
    /// `FILE_FLAG_FIRST_PIPE_INSTANCE`, which fails when any process already
    /// owns the name, so squatting can never silently succeed.
    fn create_instance(full_name: &str, sid: &str, first: bool) -> Result<HANDLE, DaemonError> {
        use windows::{
            Win32::{
                Foundation::{ERROR_ACCESS_DENIED, FALSE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree},
                Security::{
                    Authorization::{
                        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                    },
                    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
                },
                Storage::FileSystem::{
                    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
                },
                System::Pipes::{
                    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_TYPE_BYTE, PIPE_WAIT,
                },
            },
            core::{HRESULT, PCWSTR},
        };
        let fail = |operation: &'static str, source: io::Error| DaemonError::Io {
            operation,
            path: PathBuf::from(full_name),
            source,
        };
        let sddl = format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})");
        let wide_sddl: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide_sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|error| fail("secure IPC endpoint", error.into()))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: FALSE,
        };
        let wide_name: Vec<u16> = full_name.encode_utf16().chain([0]).collect();
        let flags = if first {
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED
        };
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide_name.as_ptr()),
                flags,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                64,
                65536,
                65536,
                0,
                Some(&attributes as *const _),
            )
        };
        unsafe {
            LocalFree(Some(HLOCAL(descriptor.0 as _)));
        }
        if handle == INVALID_HANDLE_VALUE {
            let error = windows::core::Error::from_thread();
            if error.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0) {
                return Err(fail(
                    "bind IPC endpoint",
                    io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "named pipe is already owned by another process",
                    ),
                ));
            }
            return Err(fail("bind IPC endpoint", error.into()));
        }
        Ok(handle)
    }

    pub fn run(mut self) -> Result<(), DaemonError> {
        use windows::Win32::{Foundation::INVALID_HANDLE_VALUE, System::Threading::CreateEventW};
        let event =
            unsafe { CreateEventW(None, true, false, None) }.map_err(|error| DaemonError::Io {
                operation: "accept IPC client",
                path: PathBuf::from(&self.full_name),
                source: error.into(),
            })?;
        // The first handle is a live listening instance from birth: a client
        // that connects before the loop starts pairs with it. Serve it as the
        // first accept instead of leaving it as a dead name-holder that would
        // swallow clients into a void.
        let first = std::mem::replace(&mut self.first, INVALID_HANDLE_VALUE);
        let outcome = self.run_loop(event, first);
        unsafe {
            let _ = CloseHandle(event);
            if self.first != INVALID_HANDLE_VALUE {
                let _ = CloseHandle(self.first);
            }
        }
        outcome
    }

    fn run_loop(&self, event: HANDLE, initial: HANDLE) -> Result<(), DaemonError> {
        use windows::{
            Win32::{
                Foundation::{CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, WAIT_OBJECT_0},
                System::{
                    IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
                    Pipes::{ConnectNamedPipe, DisconnectNamedPipe, GetNamedPipeClientProcessId},
                    RemoteDesktop::ProcessIdToSessionId,
                    Threading::{GetCurrentProcessId, ResetEvent, WaitForSingleObject},
                },
            },
            core::HRESULT,
        };
        let fail = |source: io::Error| DaemonError::Io {
            operation: "accept IPC client",
            path: PathBuf::from(&self.full_name),
            source,
        };
        let mut own_session = 0u32;
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut own_session) }
            .map_err(|error| fail(error.into()))?;
        let mut pending = Some(initial);
        while !self.shutdown.load(Ordering::Acquire) {
            let instance = match pending.take() {
                Some(handle) => handle,
                // Instance creation can fail transiently (exhausted
                // instances, scarce resources): back off and keep serving
                // existing clients rather than tearing down the daemon.
                None => match Self::create_instance(&self.full_name, &self.sid, false) {
                    Ok(instance) => instance,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(100));
                        if self.shutdown.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        continue;
                    }
                },
            };
            let mut overlapped = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            let mut connected = false;
            unsafe {
                let _ = ResetEvent(event);
            }
            match unsafe { ConnectNamedPipe(instance, Some(std::ptr::addr_of_mut!(overlapped))) } {
                Ok(()) => connected = true,
                Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => {
                    connected = true;
                }
                Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => loop {
                    let waited = unsafe { WaitForSingleObject(event, 100) };
                    if waited == WAIT_OBJECT_0 {
                        let mut unused = 0u32;
                        match unsafe {
                            GetOverlappedResult(instance, &overlapped, &mut unused, false)
                        } {
                            Ok(()) => {
                                connected = true;
                                break;
                            }
                            Err(error)
                                if error.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) =>
                            {
                                connected = true;
                                break;
                            }
                            Err(_) => break,
                        }
                    } else if self.shutdown.load(Ordering::Acquire) {
                        // Drain before the stack OVERLAPPED is freed: the
                        // kernel may still write completion status into it.
                        unsafe {
                            let _ = CancelIoEx(instance, Some(&overlapped));
                            let mut unused = 0u32;
                            let _ = GetOverlappedResult(instance, &overlapped, &mut unused, true);
                        }
                        break;
                    }
                },
                Err(_) => {}
            }
            if !connected || self.shutdown.load(Ordering::Acquire) {
                unsafe {
                    let _ = DisconnectNamedPipe(instance);
                    let _ = CloseHandle(instance);
                }
                if self.shutdown.load(Ordering::Acquire) {
                    return Ok(());
                }
                continue;
            }
            let mut client_pid = 0u32;
            let same_session =
                unsafe { GetNamedPipeClientProcessId(instance, &mut client_pid) }.is_ok() && {
                    let mut client_session = 0u32;
                    unsafe { ProcessIdToSessionId(client_pid, &mut client_session) }.is_ok()
                        && client_session == own_session
                };
            if !same_session {
                unsafe {
                    let _ = DisconnectNamedPipe(instance);
                    let _ = CloseHandle(instance);
                }
                continue;
            }
            match own_domains_protocol::pipe::PipeStream::new(instance, Duration::from_secs(30)) {
                Ok(stream) => {
                    let state = self.state.clone();
                    let shutdown = self.shutdown.clone();
                    thread::spawn(move || {
                        let mut stream = stream;
                        stream.set_no_timeout();
                        handle_client(stream, state, shutdown)
                    });
                }
                Err(_) => unsafe {
                    let _ = CloseHandle(instance);
                },
            }
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub fn request_shutdown(endpoint: &Endpoint) {
        use windows::{
            Win32::{
                Foundation::{GENERIC_READ, GENERIC_WRITE},
                Storage::FileSystem::{
                    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING,
                },
            },
            core::PCWSTR,
        };
        let wide: Vec<u16> = endpoint.display_name().encode_utf16().chain([0]).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        };
        let Ok(handle) = handle else {
            return;
        };
        if let Ok(mut stream) =
            own_domains_protocol::pipe::PipeStream::new(handle, Duration::from_secs(2))
        {
            let request = RequestEnvelope {
                protocol_version: own_domains_protocol::PROTOCOL_VERSION,
                request_id: "shutdown".into(),
                command: Command::Stop,
            };
            let _ = write_json_line(&mut stream, &request);
        }
    }
}

// The first-instance handle is an owned kernel object; transferring the
// server to the accept-loop thread by value is sound.
#[cfg(windows)]
unsafe impl Send for IpcServer {}

#[cfg(windows)]
impl Drop for IpcServer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.first);
        }
    }
}
