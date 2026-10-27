#![cfg(unix)]
use own_domains_client::{Client, ClientError, Endpoint};
use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, UnavailableRuntimeBackend};
use own_domains_protocol::{Command, CommandResult};

fn workspace_target() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
}
use std::{sync::Arc, thread, time::Duration};

#[test]
fn unavailable_daemon_is_classified() {
    let path = workspace_target().join(format!(
        "od-missing-{}.sock",
        &uuid::Uuid::new_v4().to_string()[..8]
    ));
    assert!(matches!(
        Client::connect(&Endpoint::unix(path)),
        Err(ClientError::DaemonUnavailable { .. })
    ));
}

#[test]
fn client_handshakes_and_correlates_requests() {
    let directory = workspace_target().join(format!(
        "od-client-{}",
        &uuid::Uuid::new_v4().to_string()[..8]
    ));
    let socket = directory.join("d.sock");
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(directory.join("state"))),
        Arc::new(UnavailableRuntimeBackend),
    ));
    let server = IpcServer::bind(&Endpoint::unix(socket.clone()), state).unwrap();
    let runner = thread::spawn(move || server.run().unwrap());
    let client = Client::with_timeout(&Endpoint::unix(socket), Duration::from_secs(1)).unwrap();
    assert!(matches!(
        client.request(Command::List).unwrap(),
        CommandResult::Settings(_)
    ));
    let _ = client.request(Command::Stop);
    runner.join().unwrap();
}
