use std::{sync::Arc, thread, time::Duration};
use own_domains_client::{Client, ClientError};
use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, UnavailableRuntimeBackend};
use own_domains_protocol::{Command, CommandResult};

#[test]
fn unavailable_daemon_is_classified() {
    let path = std::path::PathBuf::from("/tmp").join(format!("od-missing-{}.sock", &uuid::Uuid::new_v4().to_string()[..8]));
    assert!(matches!(Client::connect(&path), Err(ClientError::DaemonUnavailable { .. })));
}

#[test]
fn client_handshakes_and_correlates_requests() {
    let directory = std::path::PathBuf::from("/tmp").join(format!("od-client-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    let socket = directory.join("d.sock");
    let state = Arc::new(DaemonState::new(DomainService::new(SettingsStore::new(directory.join("state"))), Arc::new(UnavailableRuntimeBackend)));
    let server = IpcServer::bind(&socket, state).unwrap();
    let runner = thread::spawn(move || server.run().unwrap());
    let client = Client::with_timeout(&socket, Duration::from_secs(1)).unwrap();
    assert!(matches!(client.request(Command::List).unwrap(), CommandResult::Settings(_)));
    let _ = client.request(Command::Stop);
    runner.join().unwrap();
}
