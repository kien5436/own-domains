use crate::{Client, ClientError};
use own_domains_protocol::{Command, CommandResult, RuntimeStatus};
use std::{
    io,
    path::Path,
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("failed to launch daemon: {0}")]
    Launch(#[source] io::Error),
    #[error("daemon did not become ready before the timeout")]
    StartupTimeout,
    #[error("daemon acknowledged stop but its socket is still present; shutdown is unconfirmed")]
    ShutdownTimeout,
    #[error("daemon returned an unexpected stop result; shutdown is unconfirmed")]
    InvalidStop,
    #[error("unable to inspect daemon socket: {0}")]
    Inspect(#[source] io::Error),
}
fn absent(error: &ClientError) -> bool {
    matches!(error, ClientError::DaemonUnavailable { source, .. } if matches!(source.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused))
}
pub fn connect_or_launch(
    state_dir: &Path,
    socket: &Path,
    daemon_executable: &Path,
    timeout: Duration,
) -> Result<Client, LifecycleError> {
    let deadline = Instant::now() + timeout;
    match Client::with_timeout(socket, timeout) {
        Ok(client) => return Ok(client),
        Err(error) if absent(&error) => {}
        Err(error) => return Err(error.into()),
    }
    let mut child = ProcessCommand::new(daemon_executable)
        .arg("--state-dir")
        .arg(state_dir)
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(LifecycleError::Launch)?;
    // Reap our own child on exit; never kill a PID learned from an IPC peer.
    thread::spawn(move || {
        let _ = child.wait();
    });
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LifecycleError::StartupTimeout);
        }
        match Client::with_timeout(socket, remaining) {
            Ok(client) => return Ok(client),
            Err(error) if absent(&error) => {}
            Err(error) => return Err(error.into()),
        }
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}
pub fn stop_and_wait(
    client: &Client,
    socket: &Path,
    timeout: Duration,
) -> Result<(), LifecycleError> {
    if !matches!(
        client.request(Command::Stop)?,
        CommandResult::Runtime(RuntimeStatus::Stopped)
    ) {
        return Err(LifecycleError::InvalidStop);
    }
    let deadline = Instant::now() + timeout;
    loop {
        match std::fs::symlink_metadata(socket) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(LifecycleError::Inspect(error)),
            Ok(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(LifecycleError::ShutdownTimeout);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
