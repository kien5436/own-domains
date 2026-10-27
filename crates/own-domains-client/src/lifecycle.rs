use crate::{Client, ClientError};
use own_domains_protocol::{Command, CommandResult, Endpoint, RuntimeStatus};
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
fn absent(endpoint: &Endpoint, error: &ClientError) -> bool {
    matches!(error, ClientError::DaemonUnavailable { source, .. } if endpoint.is_absent_error(source))
}
pub fn connect_or_launch(
    state_dir: &Path,
    endpoint: &Endpoint,
    daemon_executable: &Path,
    timeout: Duration,
) -> Result<Client, LifecycleError> {
    let deadline = Instant::now() + timeout;
    match Client::with_timeout(endpoint, timeout) {
        Ok(client) => return Ok(client),
        Err(error) if absent(endpoint, &error) => {}
        Err(error) => return Err(error.into()),
    }
    let mut command = ProcessCommand::new(daemon_executable);
    command
        .arg("--state-dir")
        .arg(state_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    match endpoint.launch_argument() {
        Some((flag, value)) => {
            command.arg(flag).arg(value);
        }
        None => {
            return Err(LifecycleError::Launch(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "daemon launch cannot reproduce endpoint {}",
                    endpoint.display_name()
                ),
            )));
        }
    }
    let mut child = command.spawn().map_err(LifecycleError::Launch)?;
    // Reap our own child on exit; never kill a PID learned from an IPC peer.
    thread::spawn(move || {
        let _ = child.wait();
    });
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LifecycleError::StartupTimeout);
        }
        match Client::with_timeout(endpoint, remaining) {
            Ok(client) => return Ok(client),
            Err(error) if absent(endpoint, &error) => {}
            Err(error) => return Err(error.into()),
        }
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}
pub fn stop_and_wait(
    client: &Client,
    endpoint: &Endpoint,
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
        if !endpoint.is_present() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(LifecycleError::ShutdownTimeout);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
