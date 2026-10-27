#![cfg(unix)]
use own_domains_client::{Client, Endpoint, connect_or_launch, stop_and_wait};
use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, UnavailableRuntimeBackend};
use std::{sync::Arc, thread, time::Duration};
#[test]
fn existing_daemon_needs_no_executable_and_stop_waits_for_socket_removal() {
    let directory = std::env::temp_dir().join(format!("od-{}", uuid::Uuid::new_v4()));
    let socket = directory.join("d.sock");
    let endpoint = Endpoint::unix(socket.clone());
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&directory)),
        Arc::new(UnavailableRuntimeBackend),
    ));
    let server = IpcServer::bind(&endpoint, state).unwrap();
    let runner = thread::spawn(move || server.run().unwrap());
    let client = connect_or_launch(
        &directory,
        &endpoint,
        &directory.join("missing"),
        Duration::from_secs(1),
    )
    .unwrap();
    stop_and_wait(&client, &endpoint, Duration::from_secs(1)).unwrap();
    assert!(!socket.exists());
    runner.join().unwrap();
    assert!(Client::connect(&endpoint).is_err());
}
#[test]
fn missing_executable_returns_actionable_error_without_settings() {
    let directory = std::env::temp_dir().join(format!("od-{}", uuid::Uuid::new_v4()));
    assert!(
        connect_or_launch(
            &directory,
            &Endpoint::unix(directory.join("d.sock")),
            &directory.join("missing"),
            Duration::from_millis(100)
        )
        .err()
        .unwrap()
        .to_string()
        .contains("launch")
    );
    assert!(!directory.join("settings.json").exists());
}

#[test]
fn launched_process_that_never_binds_has_a_bounded_startup_timeout() {
    let directory = std::env::temp_dir().join(format!("od-{}", uuid::Uuid::new_v4()));
    let started = std::time::Instant::now();
    let no_op = if cfg!(target_os = "macos") {
        "/usr/bin/true"
    } else {
        "/bin/true"
    };
    let error = connect_or_launch(
        &directory,
        &Endpoint::unix(directory.join("d.sock")),
        std::path::Path::new(no_op),
        Duration::from_millis(40),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error, own_domains_client::LifecycleError::StartupTimeout),
        "unexpected lifecycle error: {error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn incompatible_daemon_is_not_replaced_by_a_new_process() {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixListener,
    };
    let directory = std::env::temp_dir().join(format!("od-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("d.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let runner = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let request: own_domains_protocol::RequestEnvelope = serde_json::from_str(&line).unwrap();
        let response = own_domains_protocol::ResponseEnvelope::ok(
            request.request_id,
            own_domains_protocol::CommandResult::Handshake {
                protocol_version: own_domains_protocol::PROTOCOL_VERSION + 1,
            },
        );
        writeln!(stream, "{}", serde_json::to_string(&response).unwrap()).unwrap();
    });
    let error = connect_or_launch(
        &directory,
        &Endpoint::unix(socket),
        &directory.join("must-not-launch"),
        Duration::from_secs(1),
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        own_domains_client::LifecycleError::Client(
            own_domains_client::ClientError::IncompatibleVersion { .. }
        )
    ));
    runner.join().unwrap();
}
