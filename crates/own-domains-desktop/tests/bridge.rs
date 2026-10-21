use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, UnavailableRuntimeBackend};
use own_domains_desktop::bridge::{BridgeError, DesktopBridge, DesktopPaths};
use own_domains_protocol::{Command, CommandResult, ProtocolError};
use std::{path::Path, sync::Arc, thread};

#[test]
fn rejects_missing_and_relative_state_paths_without_creating_settings() {
    assert!(DesktopPaths::new(None, None).is_err());
    assert!(DesktopPaths::new(Some(Path::new("relative")), None).is_err());
    let paths = DesktopPaths::new(Some(Path::new("/tmp/explicit-state")), None).unwrap();
    assert_eq!(paths.socket, Path::new("/tmp/explicit-state/daemon.sock"));
    assert!(
        DesktopPaths::new(
            Some(Path::new("/tmp/state")),
            Some(Path::new("relative.sock"))
        )
        .is_err()
    );
}

#[test]
fn bridge_preserves_errors_and_uses_only_the_selected_daemon() {
    let directory = std::env::temp_dir().join(format!("od-{}", uuid::Uuid::new_v4()));
    let socket = directory.join("d.sock");
    assert!(matches!(
        DesktopBridge::connect(&socket),
        Err(BridgeError::Unavailable { .. })
    ));
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&directory)),
        Arc::new(UnavailableRuntimeBackend),
    ));
    let server = IpcServer::bind(&socket, state).unwrap();
    let runner = thread::spawn(move || server.run().unwrap());
    let bridge = DesktopBridge::connect(&socket).unwrap();
    assert!(matches!(
        bridge.request(Command::Snapshot).unwrap(),
        CommandResult::Snapshot(_)
    ));
    assert!(matches!(
        bridge.request(Command::Handshake),
        Err(BridgeError::InvalidMessage { .. })
    ));
    assert!(matches!(
        bridge.request(Command::Subscribe {
            after_sequence: None
        }),
        Err(BridgeError::InvalidMessage { .. })
    ));
    let error = bridge
        .request(Command::UpdateNetwork {
            lan_address: "192.168.1.2".into(),
            upstream_dns: vec!["1.1.1.1".into()],
            expected_revision: 99,
        })
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error).unwrap()["error"]["code"],
        "revisionConflict"
    );
    assert!(matches!(
        bridge.request(Command::Start),
        Err(BridgeError::Protocol {
            error: ProtocolError::RuntimeUnavailable { .. }
        })
    ));
    bridge.request(Command::Stop).unwrap();
    runner.join().unwrap();
}
