use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, UnavailableRuntimeBackend};
use own_domains_desktop::{bridge::DesktopBridge, subscription::Subscriptions};
use own_domains_protocol::Command;
use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};
#[test]
fn replacing_and_removing_idle_streams_joins_their_workers() {
    let directory = std::env::temp_dir().join(format!("od-{}", uuid::Uuid::new_v4()));
    let socket = directory.join("d.sock");
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&directory)),
        Arc::new(UnavailableRuntimeBackend),
    ));
    let server = IpcServer::bind(&socket, state).unwrap();
    let runner = thread::spawn(move || server.run().unwrap());
    let bridge = DesktopBridge::connect(&socket).unwrap();
    let mut subscriptions = Subscriptions::default();
    let (tx, rx) = mpsc::channel();
    let callback = Arc::new(move |id, event| {
        let _ = tx.send((id, event));
    });
    let first = subscriptions
        .replace(&bridge, None, callback.clone())
        .unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().0, first);
    let second = subscriptions.replace(&bridge, None, callback).unwrap();
    assert_ne!(first, second);
    assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().0, second);
    subscriptions.remove(first).unwrap(); // stale cleanup must not remove the new stream
    subscriptions.remove(second).unwrap();
    bridge.request(Command::Stop).unwrap();
    runner.join().unwrap();
}
