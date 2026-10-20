use std::{fs, io::{BufRead, BufReader, Write}, os::unix::{fs::PermissionsExt, net::UnixStream}, sync::Arc, thread, time::Duration};
use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, UnavailableRuntimeBackend};
use own_domains_protocol::{Command, CommandResult, RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};

fn setup() -> (std::path::PathBuf, Arc<DaemonState>, IpcServer) {
    let directory = std::path::PathBuf::from("/tmp").join(format!("od-ipc-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    let socket = directory.join("daemon.sock");
    let state = Arc::new(DaemonState::new(DomainService::new(SettingsStore::new(directory.join("state"))), Arc::new(UnavailableRuntimeBackend)));
    let server = IpcServer::bind(&socket, state.clone()).unwrap();
    (socket, state, server)
}

#[test]
fn socket_is_private_and_removed_after_shutdown() {
    let (socket, _, server) = setup();
    assert_eq!(fs::metadata(socket.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
    server.shutdown();
    drop(server);
    assert!(!socket.exists());
}

#[test]
fn malformed_client_does_not_break_following_requests() {
    let (socket, _, server) = setup();
    let runner = thread::spawn(move || server.run().unwrap());
    let mut bad = UnixStream::connect(&socket).unwrap();
    bad.write_all(b"not-json\n").unwrap();
    drop(bad);
    let mut good = UnixStream::connect(&socket).unwrap();
    let request = RequestEnvelope { protocol_version: PROTOCOL_VERSION, request_id: "healthy".into(), command: Command::Handshake };
    writeln!(good, "{}", serde_json::to_string(&request).unwrap()).unwrap();
    let mut line = String::new();
    BufReader::new(&good).read_line(&mut line).unwrap();
    let response: ResponseEnvelope = serde_json::from_str(&line).unwrap();
    assert!(matches!(response.result, Ok(CommandResult::Handshake { .. })));
    IpcServer::request_shutdown(&socket);
    runner.join().unwrap();
    thread::sleep(Duration::from_millis(10));
}
