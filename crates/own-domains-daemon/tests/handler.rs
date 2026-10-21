use own_domains_core::{
    ApplyResult, ApplyStatus, Diagnostic, DomainService, MappingInput, Settings, SettingsStore,
};
use own_domains_daemon::{
    DaemonState, EventBroker, RuntimeBackend, RuntimeError, UnavailableRuntimeBackend,
};
use own_domains_protocol::{
    Command, CommandResult, Event, PROTOCOL_VERSION, ProtocolError, RequestEnvelope, RuntimeStatus,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

struct RecordingRuntime {
    outcomes: Mutex<VecDeque<Result<(), &'static str>>>,
    revisions: Mutex<Vec<u64>>,
}

impl RecordingRuntime {
    fn new(outcomes: impl IntoIterator<Item = Result<(), &'static str>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            revisions: Mutex::new(vec![]),
        }
    }

    fn revisions(&self) -> Vec<u64> {
        self.revisions.lock().unwrap().clone()
    }
}

impl RuntimeBackend for RecordingRuntime {
    fn apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError> {
        self.revisions.lock().unwrap().push(settings.revision);
        match self.outcomes.lock().unwrap().pop_front().unwrap_or(Ok(())) {
            Ok(()) => Ok(ApplyResult {
                desired_revision: settings.revision,
                applied_revision: settings.revision,
                status: ApplyStatus::Applied,
                message: None,
            }),
            Err(message) => Err(RuntimeError::Failure(message.into())),
        }
    }

    fn stop(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn inspect(&self) -> Result<RuntimeStatus, RuntimeError> {
        Ok(RuntimeStatus::Running)
    }
    fn diagnose(&self, _settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError> {
        Ok(vec![])
    }
}

fn state() -> DaemonState {
    let path = std::env::temp_dir().join(format!("own-domains-state-{}", uuid::Uuid::new_v4()));
    DaemonState::new(
        DomainService::new(SettingsStore::new(path)),
        Arc::new(UnavailableRuntimeBackend),
    )
}

fn request(command: Command) -> RequestEnvelope {
    RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: "r1".into(),
        command,
    }
}

#[test]
fn mutations_publish_only_after_success() {
    let state = state();
    let subscription = state
        .events()
        .subscribe(Some(0), || state.snapshot().unwrap());
    let response = state.handle(request(Command::Create {
        input: MappingInput {
            id: None,
            hostname: "shop.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        },
        expected_revision: 0,
    }));
    assert!(
        matches!(response.result, Ok(CommandResult::Mutation(ref result)) if result.settings.revision == 1 && result.apply.is_none())
    );
    assert!(matches!(
        subscription
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .event,
        Event::SettingsChanged { revision: 1 }
    ));
    let rejected = state.handle(request(Command::Create {
        input: MappingInput {
            id: None,
            hostname: "shop.own".into(),
            upstream_port: 3001,
            tls: false,
            enabled: true,
        },
        expected_revision: 1,
    }));
    assert!(matches!(
        rejected.result,
        Err(ProtocolError::DuplicateHostname { .. })
    ));
    assert!(
        subscription
            .recv_timeout(Duration::from_millis(20))
            .is_err()
    );
}

#[test]
fn stopped_mutation_is_saved_without_applying() {
    let runtime = Arc::new(RecordingRuntime::new([]));
    let path = std::env::temp_dir().join(format!("own-domains-state-{}", uuid::Uuid::new_v4()));
    let state = DaemonState::new(
        DomainService::new(SettingsStore::new(path)),
        runtime.clone(),
    );
    let response = state.handle(request(Command::Create {
        input: MappingInput {
            id: None,
            hostname: "shop.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        },
        expected_revision: 0,
    }));

    assert!(
        matches!(response.result, Ok(CommandResult::Mutation(ref result)) if result.settings.revision == 1 && result.apply.is_none())
    );
    assert!(runtime.revisions().is_empty());
    assert_eq!(
        state.snapshot().unwrap().runtime_status,
        RuntimeStatus::Stopped
    );
}

#[test]
fn live_mutation_failure_preserves_saved_revision_and_recovers_on_retry() {
    let runtime = Arc::new(RecordingRuntime::new([
        Ok(()),
        Err("reload failed"),
        Ok(()),
    ]));
    let path = std::env::temp_dir().join(format!("own-domains-state-{}", uuid::Uuid::new_v4()));
    let state = DaemonState::new(
        DomainService::new(SettingsStore::new(path)),
        runtime.clone(),
    );
    assert!(matches!(
        state.handle(request(Command::Start)).result,
        Ok(CommandResult::Apply(_))
    ));
    let after_start = state.events().latest_sequence();
    let subscription = state
        .events()
        .subscribe(Some(after_start), || state.snapshot().unwrap());

    let failed = state.handle(request(Command::Create {
        input: MappingInput {
            id: None,
            hostname: "shop.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        },
        expected_revision: 0,
    }));
    assert!(
        matches!(failed.result, Ok(CommandResult::Mutation(ref result)) if matches!(result.apply, Some(ApplyResult { desired_revision: 1, applied_revision: 0, status: ApplyStatus::Error, .. })))
    );
    let degraded = state.snapshot().unwrap();
    assert_eq!(degraded.settings.revision, 1);
    assert_eq!(degraded.desired_revision, 1);
    assert_eq!(degraded.applied_revision, 0);
    assert_eq!(degraded.runtime_status, RuntimeStatus::Degraded);
    assert!(matches!(
        subscription
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .event,
        Event::SettingsChanged { revision: 1 }
    ));
    assert!(matches!(
        subscription
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .event,
        Event::RuntimeChanged {
            status: RuntimeStatus::Degraded
        }
    ));
    assert!(matches!(
        subscription
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .event,
        Event::DiagnosticsChanged { .. }
    ));

    let recovered = state.handle(request(Command::UpdateNetwork {
        lan_address: "192.168.1.20".into(),
        upstream_dns: vec!["1.1.1.1".into()],
        expected_revision: 1,
    }));
    assert!(
        matches!(recovered.result, Ok(CommandResult::Mutation(ref result)) if matches!(result.apply, Some(ApplyResult { desired_revision: 2, applied_revision: 2, status: ApplyStatus::Applied, .. })))
    );
    let running = state.snapshot().unwrap();
    assert_eq!(running.applied_revision, 2);
    assert_eq!(running.runtime_status, RuntimeStatus::Running);
    assert_eq!(runtime.revisions(), vec![0, 1, 2]);
}

#[test]
fn unavailable_runtime_never_claims_start_success() {
    let state = state();
    let response = state.handle(request(Command::Start));
    assert!(matches!(
        response.result,
        Err(ProtocolError::RuntimeUnavailable { .. })
    ));
    assert_eq!(
        state.snapshot().unwrap().runtime_status,
        RuntimeStatus::Error
    );
}

#[test]
fn event_sequences_are_monotonic_and_old_subscribers_resync() {
    let broker = EventBroker::with_capacity(2);
    for revision in 1..=3 {
        broker.publish(Event::SettingsChanged { revision });
    }
    let subscription = broker.subscribe(Some(0), || own_domains_protocol::StateSnapshot {
        settings: Default::default(),
        runtime_status: RuntimeStatus::Stopped,
        desired_revision: 0,
        applied_revision: 0,
        diagnostics: vec![],
        latest_event_sequence: broker.latest_sequence(),
    });
    let event = subscription.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(event.sequence, 3);
    assert!(matches!(event.event, Event::ResyncRequired { .. }));
    broker.publish(Event::SettingsChanged { revision: 4 });
    assert_eq!(
        subscription
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .sequence,
        4
    );
}
