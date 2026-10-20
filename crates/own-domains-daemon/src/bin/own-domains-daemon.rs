use std::{path::PathBuf, sync::Arc};
use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, ProcessLock, UnavailableRuntimeBackend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut state_dir = PathBuf::from(".own-domains");
    let mut socket = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--state-dir" => state_dir = PathBuf::from(args.next().ok_or("--state-dir requires a path")?),
            "--socket" => socket = Some(PathBuf::from(args.next().ok_or("--socket requires a path")?)),
            "--settings-only" => {},
            other => return Err(format!("unknown option: {other}").into()),
        }
    }
    let socket = socket.unwrap_or_else(|| state_dir.join("daemon.sock"));
    let _lock = ProcessLock::acquire(&state_dir)?;
    let state = Arc::new(DaemonState::new(DomainService::new(SettingsStore::new(&state_dir)), Arc::new(UnavailableRuntimeBackend)));
    IpcServer::bind(&socket, state)?.run()?;
    Ok(())
}
