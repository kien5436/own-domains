#![cfg(windows)]

//! Real named-pipe transport evidence (Gate C prerequisites).
//!
//! These tests run only on Windows, against a live `IpcServer` and the real
//! client: bind ownership, handshake, commands, the event stream,
//! cancellation, and shutdown with name release. They fail against the typed
//! stubs, so a green run proves the Win32 transport, not the scaffolding.

use own_domains_client::{Client, Endpoint, stop_and_wait};
use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonError, DaemonState, IpcServer, UnavailableRuntimeBackend};
use own_domains_protocol::{Command, CommandResult, Event};
use std::{sync::Arc, thread, time::Duration};

fn setup() -> (Endpoint, Arc<DaemonState>) {
    let directory = std::env::temp_dir().join(format!("od-pipe-{}", uuid::Uuid::new_v4()));
    let endpoint = Endpoint::default_for_state_dir(&directory).unwrap();
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&directory)),
        Arc::new(UnavailableRuntimeBackend),
    ));
    (endpoint, state)
}

#[test]
fn second_bind_fails_while_the_first_owns_the_name() {
    let (endpoint, state) = setup();
    let _first = IpcServer::bind(&endpoint, state.clone()).unwrap();
    match IpcServer::bind(&endpoint, state) {
        Err(DaemonError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::AddrInUse);
        }
        Err(other) => panic!("second bind failed with the wrong error: {other}"),
        Ok(_) => panic!("second bind must be rejected"),
    }
}

#[test]
fn client_roundtrips_commands_over_the_pipe() {
    let (endpoint, state) = setup();
    let server = IpcServer::bind(&endpoint, state).unwrap();
    let runner = thread::spawn(move || server.run());
    let client = Client::connect(&endpoint).unwrap();
    assert!(matches!(
        client.request(Command::Snapshot).unwrap(),
        CommandResult::Snapshot(_)
    ));
    IpcServer::request_shutdown(&endpoint);
    runner.join().unwrap().unwrap();
    assert!(!endpoint.is_present());
}

#[test]
fn subscription_streams_published_events_until_cancelled() {
    let (endpoint, state) = setup();
    let server = IpcServer::bind(&endpoint, state.clone()).unwrap();
    let runner = thread::spawn(move || server.run());
    let client = Client::with_timeout(&endpoint, Duration::from_secs(10)).unwrap();
    let mut stream = client.subscribe(None).unwrap();
    let first = stream.recv().unwrap();
    assert!(matches!(first.event, Event::Snapshot(_)));
    state
        .events()
        .publish(Event::SettingsChanged { revision: 7 });
    let second = stream.recv().unwrap();
    assert_eq!(second.sequence, first.sequence + 1);
    assert!(matches!(
        second.event,
        Event::SettingsChanged { revision: 7 }
    ));
    stream.cancellation_handle().unwrap().cancel().unwrap();
    assert!(stream.recv().is_err());
    IpcServer::request_shutdown(&endpoint);
    runner.join().unwrap().unwrap();
}

#[test]
fn stop_waits_for_shutdown_and_releases_the_name() {
    let (endpoint, state) = setup();
    let server = IpcServer::bind(&endpoint, state.clone()).unwrap();
    let runner = thread::spawn(move || server.run());
    let client = Client::with_timeout(&endpoint, Duration::from_secs(10)).unwrap();
    stop_and_wait(&client, &endpoint, Duration::from_secs(10)).unwrap();
    runner.join().unwrap().unwrap();
    assert!(!endpoint.is_present());
    IpcServer::bind(&endpoint, state).unwrap();
}

#[test]
fn connect_to_a_missing_pipe_is_absence_not_failure() {
    let (endpoint, _) = setup();
    assert!(matches!(
        Client::connect(&endpoint),
        Err(own_domains_client::ClientError::DaemonUnavailable { .. })
    ));
}
