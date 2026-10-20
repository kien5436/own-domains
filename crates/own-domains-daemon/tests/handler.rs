use std::{sync::Arc, time::Duration};
use own_domains_core::{DomainService, MappingInput, SettingsStore};
use own_domains_daemon::{DaemonState, EventBroker, UnavailableRuntimeBackend};
use own_domains_protocol::{Command, CommandResult, Event, ProtocolError, RequestEnvelope, RuntimeStatus, PROTOCOL_VERSION};

fn state() -> DaemonState {
    let path = std::env::temp_dir().join(format!("own-domains-state-{}", uuid::Uuid::new_v4()));
    DaemonState::new(DomainService::new(SettingsStore::new(path)), Arc::new(UnavailableRuntimeBackend))
}

fn request(command: Command) -> RequestEnvelope {
    RequestEnvelope { protocol_version: PROTOCOL_VERSION, request_id: "r1".into(), command }
}

#[test]
fn mutations_publish_only_after_success() {
    let state = state();
    let subscription = state.events().subscribe(Some(0), || state.snapshot().unwrap());
    let response = state.handle(request(Command::Create { input: MappingInput { id: None, hostname: "shop.own".into(), upstream_port: 3000, tls: false, enabled: true }, expected_revision: 0 }));
    assert!(matches!(response.result, Ok(CommandResult::Settings(ref settings)) if settings.revision == 1));
    assert!(matches!(subscription.recv_timeout(Duration::from_secs(1)).unwrap().event, Event::SettingsChanged { revision: 1 }));
    let rejected = state.handle(request(Command::Create { input: MappingInput { id: None, hostname: "shop.own".into(), upstream_port: 3001, tls: false, enabled: true }, expected_revision: 1 }));
    assert!(matches!(rejected.result, Err(ProtocolError::DuplicateHostname { .. })));
    assert!(subscription.recv_timeout(Duration::from_millis(20)).is_err());
}

#[test]
fn unavailable_runtime_never_claims_start_success() {
    let state = state();
    let response = state.handle(request(Command::Start));
    assert!(matches!(response.result, Err(ProtocolError::RuntimeUnavailable { .. })));
    assert_eq!(state.snapshot().unwrap().runtime_status, RuntimeStatus::Error);
}

#[test]
fn event_sequences_are_monotonic_and_old_subscribers_resync() {
    let broker = EventBroker::with_capacity(2);
    for revision in 1..=3 { broker.publish(Event::SettingsChanged { revision }); }
    let subscription = broker.subscribe(Some(0), || own_domains_protocol::StateSnapshot { settings: Default::default(), runtime_status: RuntimeStatus::Stopped, desired_revision: 0, applied_revision: 0, diagnostics: vec![], latest_event_sequence: broker.latest_sequence() });
    let event = subscription.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(event.sequence, 4);
    assert!(matches!(event.event, Event::ResyncRequired { .. }));
}
