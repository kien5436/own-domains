use std::{fs, io::{self, Read, Write}, os::unix::{fs::{FileTypeExt, MetadataExt, PermissionsExt}, net::{UnixListener, UnixStream}}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, thread, time::Duration};
use own_domains_protocol::{Command, RequestEnvelope};
use crate::{DaemonError, DaemonState};

const MAX_FRAME: usize = 1024 * 1024;

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
    state: Arc<DaemonState>,
    shutdown: Arc<AtomicBool>,
}

impl IpcServer {
    pub fn bind(path: &Path, state: Arc<DaemonState>) -> Result<Self, DaemonError> {
        let parent = path.parent().ok_or_else(|| DaemonError::InvalidSocketPath { path: path.to_path_buf() })?;
        fs::create_dir_all(parent).map_err(|source| io_error("create IPC directory", parent, source))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|source| io_error("secure IPC directory", parent, source))?;
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() || UnixStream::connect(path).is_ok() { return Err(DaemonError::InvalidSocketPath { path: path.to_path_buf() }); }
            fs::remove_file(path).map_err(|source| io_error("remove stale IPC socket", path, source))?;
        }
        let listener = UnixListener::bind(path).map_err(|source| io_error("bind IPC socket", path, source))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| io_error("secure IPC socket", path, source))?;
        let metadata = fs::metadata(path).map_err(|source| io_error("inspect IPC socket", path, source))?;
        listener.set_nonblocking(true).map_err(|source| io_error("configure IPC socket", path, source))?;
        Ok(Self { listener, path: path.to_path_buf(), identity: (metadata.dev(), metadata.ino()), state, shutdown: Arc::new(AtomicBool::new(false)) })
    }

    pub fn run(self) -> Result<(), DaemonError> {
        while !self.shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let state = self.state.clone();
                    let shutdown = self.shutdown.clone();
                    thread::spawn(move || handle_client(stream, state, shutdown));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                Err(error) if matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted) => continue,
                Err(source) => return Err(io_error("accept IPC client", &self.path, source)),
            }
        }
        Ok(())
    }

    pub fn shutdown(&self) { self.shutdown.store(true, Ordering::Release); }

    pub fn request_shutdown(path: &Path) {
        if let Ok(mut stream) = UnixStream::connect(path) {
            let request = RequestEnvelope { protocol_version: own_domains_protocol::PROTOCOL_VERSION, request_id: "shutdown".into(), command: Command::Stop };
            if let Ok(frame) = serde_json::to_vec(&request) { let _ = stream.write_all(&frame); let _ = stream.write_all(b"\n"); }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::metadata(&self.path)
            && (metadata.dev(), metadata.ino()) == self.identity
        { let _ = fs::remove_file(&self.path); }
    }
}

fn handle_client(mut stream: UnixStream, state: Arc<DaemonState>, shutdown: Arc<AtomicBool>) {
    loop {
        let frame = match read_frame(&mut stream) { Ok(Some(frame)) => frame, _ => return };
        let request: RequestEnvelope = match serde_json::from_slice(&frame) { Ok(request) => request, Err(_) => return };
        let subscribing = match request.command { Command::Subscribe { after_sequence } => Some(after_sequence), _ => None };
        let stopping = matches!(request.command, Command::Stop);
        let response = state.handle(request);
        if stopping && response.result.is_ok() { shutdown.store(true, Ordering::Release); }
        if write_json_line(&mut stream, &response).is_err() { return; }
        if let Some(after_sequence) = subscribing {
            let subscription = state.events().subscribe(after_sequence, || state.snapshot().unwrap_or_else(|_| empty_snapshot()));
            while let Ok(event) = subscription.recv() { if write_json_line(&mut stream, &event).is_err() { break; } }
            return;
        }
    }
}

fn read_frame(stream: &mut UnixStream) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    let mut byte = [0];
    loop {
        match stream.read(&mut byte)? {
            0 if frame.is_empty() => return Ok(None),
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "IPC frame is missing newline")),
            _ if byte[0] == b'\n' => return Ok(Some(frame)),
            _ => { frame.push(byte[0]); if frame.len() > MAX_FRAME { return Err(io::Error::new(io::ErrorKind::InvalidData, "IPC frame exceeds 1 MiB")); } }
        }
    }
}

fn write_json_line(stream: &mut UnixStream, value: &impl serde::Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}

fn empty_snapshot() -> own_domains_protocol::StateSnapshot { own_domains_protocol::StateSnapshot { settings: Default::default(), runtime_status: own_domains_protocol::RuntimeStatus::Error, desired_revision: 0, applied_revision: 0, diagnostics: vec![], latest_event_sequence: 0 } }

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> DaemonError { DaemonError::Io { operation, path: path.to_path_buf(), source } }
