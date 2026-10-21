use crate::{EventBroker, RuntimeBackend, RuntimeError};
use own_domains_core::{ApplyResult, ApplyStatus, CoreError, DomainService, Settings};
use own_domains_protocol::{
    Command, CommandResult, Event, MutationResult, PROTOCOL_VERSION, ProtocolError,
    RequestEnvelope, ResponseEnvelope, RuntimeStatus, StateSnapshot, validate_version,
};
use std::sync::{Arc, Mutex};

struct Inner {
    service: DomainService,
    runtime_status: RuntimeStatus,
    runtime_active: bool,
    applied_revision: u64,
}

pub struct DaemonState {
    inner: Mutex<Inner>,
    runtime: Arc<dyn RuntimeBackend>,
    events: EventBroker,
}

impl DaemonState {
    pub fn new(service: DomainService, runtime: Arc<dyn RuntimeBackend>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                service,
                runtime_status: RuntimeStatus::Stopped,
                runtime_active: false,
                applied_revision: 0,
            }),
            runtime,
            events: EventBroker::default(),
        }
    }

    pub fn events(&self) -> &EventBroker {
        &self.events
    }

    pub fn snapshot(&self) -> Result<StateSnapshot, ProtocolError> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let settings = inner.service.list().map_err(map_core_error)?;
        let diagnostics = self.runtime.diagnose(&settings).unwrap_or_default();
        Ok(StateSnapshot {
            desired_revision: settings.revision,
            applied_revision: inner.applied_revision,
            settings,
            runtime_status: inner.runtime_status.clone(),
            diagnostics,
            latest_event_sequence: self.events.latest_sequence(),
        })
    }

    pub fn handle(&self, request: RequestEnvelope) -> ResponseEnvelope {
        if let Err(error) = validate_version(request.protocol_version) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        let result = self.handle_command(request.command);
        match result {
            Ok(result) => ResponseEnvelope::ok(request.request_id, result),
            Err(error) => ResponseEnvelope::error(request.request_id, error),
        }
    }

    fn handle_command(&self, command: Command) -> Result<CommandResult, ProtocolError> {
        if matches!(command, Command::Snapshot) {
            return self.snapshot().map(CommandResult::Snapshot);
        }
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match command {
            Command::Handshake => Ok(CommandResult::Handshake {
                protocol_version: PROTOCOL_VERSION,
            }),
            Command::List => inner
                .service
                .list()
                .map(CommandResult::Settings)
                .map_err(map_core_error),
            Command::Create {
                input,
                expected_revision,
            } => {
                let result = inner.service.create(input, expected_revision);
                mutate(&self.events, self.runtime.as_ref(), &mut inner, result)
            }
            Command::Update {
                id,
                patch,
                expected_revision,
            } => {
                let result = inner.service.update(&id, patch, expected_revision);
                mutate(&self.events, self.runtime.as_ref(), &mut inner, result)
            }
            Command::Remove {
                id,
                expected_revision,
            } => {
                let result = inner.service.remove(&id, expected_revision);
                mutate(&self.events, self.runtime.as_ref(), &mut inner, result)
            }
            Command::UpdateNetwork {
                lan_address,
                upstream_dns,
                expected_revision,
            } => {
                let result =
                    inner
                        .service
                        .update_network(lan_address, upstream_dns, expected_revision);
                mutate(&self.events, self.runtime.as_ref(), &mut inner, result)
            }
            Command::Start => {
                set_status(&self.events, &mut inner, RuntimeStatus::Starting);
                let settings = inner.service.list().map_err(map_core_error)?;
                match self.runtime.apply(&settings) {
                    Ok(applied) => {
                        inner.runtime_active = true;
                        inner.applied_revision = applied.applied_revision;
                        set_status(&self.events, &mut inner, RuntimeStatus::Running);
                        Ok(CommandResult::Apply(applied))
                    }
                    Err(error) => {
                        let status = if inner.runtime_active {
                            RuntimeStatus::Degraded
                        } else {
                            RuntimeStatus::Error
                        };
                        set_status(&self.events, &mut inner, status);
                        Err(map_runtime_error(error))
                    }
                }
            }
            Command::Stop => {
                set_status(&self.events, &mut inner, RuntimeStatus::Stopping);
                match self.runtime.stop() {
                    Ok(()) => {
                        inner.runtime_active = false;
                        set_status(&self.events, &mut inner, RuntimeStatus::Stopped);
                        Ok(CommandResult::Runtime(RuntimeStatus::Stopped))
                    }
                    Err(error) => {
                        set_status(&self.events, &mut inner, RuntimeStatus::Error);
                        Err(map_runtime_error(error))
                    }
                }
            }
            Command::Doctor => {
                let settings = inner.service.list().map_err(map_core_error)?;
                self.runtime
                    .diagnose(&settings)
                    .map(CommandResult::Diagnostics)
                    .map_err(map_runtime_error)
            }
            Command::Subscribe { .. } => Ok(CommandResult::Subscribed),
            Command::Snapshot => unreachable!(),
        }
    }
}

fn mutate(
    events: &EventBroker,
    runtime: &dyn RuntimeBackend,
    inner: &mut Inner,
    result: Result<Settings, CoreError>,
) -> Result<CommandResult, ProtocolError> {
    let settings = result.map_err(map_core_error)?;
    let apply = if inner.runtime_active {
        let result = match runtime.apply(&settings) {
            Ok(applied) => {
                inner.applied_revision = applied.applied_revision;
                inner.runtime_status = RuntimeStatus::Running;
                applied
            }
            Err(error) => {
                inner.runtime_status = RuntimeStatus::Degraded;
                ApplyResult {
                    desired_revision: settings.revision,
                    applied_revision: inner.applied_revision,
                    status: ApplyStatus::Error,
                    message: Some(error.to_string()),
                }
            }
        };
        Some(result)
    } else {
        None
    };
    let diagnostics = runtime.diagnose(&settings).unwrap_or_default();
    events.publish(Event::SettingsChanged {
        revision: settings.revision,
    });
    if apply.is_some() {
        events.publish(Event::RuntimeChanged {
            status: inner.runtime_status.clone(),
        });
        events.publish(Event::DiagnosticsChanged { diagnostics });
    }
    Ok(CommandResult::Mutation(MutationResult { settings, apply }))
}

fn set_status(events: &EventBroker, inner: &mut Inner, status: RuntimeStatus) {
    inner.runtime_status = status.clone();
    events.publish(Event::RuntimeChanged { status });
}

fn map_runtime_error(error: RuntimeError) -> ProtocolError {
    match error {
        RuntimeError::Unavailable(message) => ProtocolError::RuntimeUnavailable { message },
        RuntimeError::Failure(message) => ProtocolError::RuntimeFailure { message },
    }
}

fn map_core_error(error: CoreError) -> ProtocolError {
    match error {
        CoreError::Validation { message } => ProtocolError::Validation { message },
        CoreError::RevisionConflict { expected, actual } => {
            ProtocolError::RevisionConflict { expected, actual }
        }
        CoreError::NotFound { entity, id } => ProtocolError::NotFound {
            message: format!("{entity} not found: {id}"),
        },
        CoreError::DuplicateHostname { hostname } => ProtocolError::DuplicateHostname { hostname },
        other => ProtocolError::Storage {
            message: other.to_string(),
        },
    }
}
