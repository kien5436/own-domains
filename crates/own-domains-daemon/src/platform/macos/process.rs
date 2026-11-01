//! macOS process supervision.
//!
//! The shared core lives in [`crate::platform::unix::process`]; only the
//! [`ProcessSupervisor`] orchestration and [`FileProcessRecords`] remain here
//! until the shared runtime paths land.

use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use super::RuntimePaths;
use super::{
    ChildProcess, OwnedProcessRecord, ProcessComponent, ProcessError, ProcessHost, ProcessIdentity,
    ProcessRecords, ProcessState, RetryPolicy, SIGKILL, SIGTERM, SpawnSpec, Waiter,
};
use crate::platform::unix::process::canonical_program;
use crate::probe::ReadinessProbe;

const RECORD_SCHEMA_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;

pub struct ProcessSupervisor<'a> {
    paths: RuntimePaths,
    host: &'a dyn ProcessHost,
    records: &'a dyn ProcessRecords,
    waiter: &'a dyn Waiter,
    children: Mutex<HashMap<ProcessComponent, Box<dyn ChildProcess>>>,
    pub readiness: RetryPolicy,
    pub graceful_stop: RetryPolicy,
    pub forced_stop: RetryPolicy,
}

impl<'a> ProcessSupervisor<'a> {
    pub fn new(
        paths: RuntimePaths,
        host: &'a dyn ProcessHost,
        records: &'a dyn ProcessRecords,
        waiter: &'a dyn Waiter,
    ) -> Self {
        Self {
            paths,
            host,
            records,
            waiter,
            children: Mutex::new(HashMap::new()),
            readiness: RetryPolicy {
                attempts: 50,
                interval: Duration::from_millis(100),
            },
            graceful_stop: RetryPolicy {
                attempts: 30,
                interval: Duration::from_millis(100),
            },
            forced_stop: RetryPolicy {
                attempts: 20,
                interval: Duration::from_millis(100),
            },
        }
    }

    pub fn start(
        &self,
        component: ProcessComponent,
        configuration: &Path,
    ) -> Result<OwnedProcessRecord, ProcessError> {
        if let Some(record) = self.records.read(component).map_err(ProcessError::Record)? {
            return Err(ProcessError::AlreadyRunning {
                component: component.name(),
                pid: record.pid,
            });
        }

        let spec = self.spawn_spec(component, configuration);
        let mut child = self
            .host
            .spawn(&spec)
            .map_err(|message| ProcessError::Spawn {
                component: component.name(),
                message,
            })?;
        let pid = child.pid();
        let identity = match self.host.identity(pid) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                let _ = child.signal(SIGKILL);
                let _ = child.wait();
                return Err(ProcessError::Identity {
                    component: component.name(),
                    message: "the spawned process disappeared before it could be verified".into(),
                });
            }
            Err(message) => {
                let _ = child.signal(SIGKILL);
                let _ = child.wait();
                return Err(ProcessError::Identity {
                    component: component.name(),
                    message,
                });
            }
        };
        if identity.executable != spec.program || identity.arguments != spec.arguments {
            let _ = child.signal(SIGKILL);
            let _ = child.wait();
            return Err(ProcessError::Identity {
                component: component.name(),
                message: "the spawned process does not match the requested executable or arguments"
                    .into(),
            });
        }

        let record = OwnedProcessRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            component,
            pid,
            process_group: identity.process_group,
            executable: identity.executable,
            arguments: identity.arguments,
            start_identity: identity.start_identity,
            log_path: spec.log_path,
        };
        if let Err(message) = self.records.write(&record) {
            let _ = child.signal(SIGKILL);
            let _ = child.wait();
            return Err(ProcessError::Record(message));
        }
        self.children.lock().unwrap().insert(component, child);
        Ok(record)
    }

    pub fn inspect_component(
        &self,
        component: ProcessComponent,
    ) -> Result<ProcessState, ProcessError> {
        let Some(record) = self.records.read(component).map_err(ProcessError::Record)? else {
            return Ok(ProcessState::Stopped);
        };
        match self.host.identity(record.pid) {
            Ok(None) => Ok(ProcessState::Stale {
                record,
                reason: "the recorded process is no longer running".into(),
            }),
            Ok(Some(identity)) if record.matches(&identity) => Ok(ProcessState::Running(record)),
            Ok(Some(_)) => Ok(ProcessState::Ambiguous {
                record,
                reason: "the process identity does not match the ownership record".into(),
            }),
            Err(message) => Ok(ProcessState::Ambiguous {
                record,
                reason: format!("the process identity could not be inspected: {message}"),
            }),
        }
    }

    pub fn stop_component(&self, component: ProcessComponent) -> Result<(), ProcessError> {
        let Some(record) = self.records.read(component).map_err(ProcessError::Record)? else {
            return Ok(());
        };

        let child = self.children.lock().unwrap().remove(&component);
        if let Some(mut child) = child {
            self.stop_live(component, child.as_mut())?;
        } else {
            self.stop_recovered(component, &record)?;
        }

        self.verify_no_owned_listeners(component, &record)?;
        self.records
            .remove(component)
            .map_err(ProcessError::Record)?;
        Ok(())
    }

    pub fn wait_for_readiness(&self, probe: &dyn ReadinessProbe) -> Result<(), ProcessError> {
        let mut last = String::from("the probe did not run");
        for attempt in 0..self.readiness.attempts {
            match probe.check() {
                Ok(()) => return Ok(()),
                Err(message) => last = message,
            }
            if let Some(delay) = self.readiness.delay_before_next(attempt) {
                self.waiter.sleep(delay);
            }
        }
        Err(ProcessError::Readiness {
            component: probe.component(),
            message: last,
        })
    }

    fn stop_live(
        &self,
        component: ProcessComponent,
        child: &mut dyn ChildProcess,
    ) -> Result<(), ProcessError> {
        child
            .signal(SIGTERM)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGTERM failed: {message}"),
            })?;
        if self.wait_for_exit(
            || {
                child
                    .try_wait()
                    .map(|status| status.is_some())
                    .map_err(|message| ProcessError::Stop {
                        component: component.name(),
                        message,
                    })
            },
            self.graceful_stop,
        )? {
            return Ok(());
        }
        child
            .signal(SIGKILL)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGKILL failed: {message}"),
            })?;
        if self.wait_for_exit(
            || {
                child
                    .try_wait()
                    .map(|status| status.is_some())
                    .map_err(|message| ProcessError::Stop {
                        component: component.name(),
                        message,
                    })
            },
            self.forced_stop,
        )? {
            return Ok(());
        }
        Err(ProcessError::Stop {
            component: component.name(),
            message: "the process did not exit after SIGKILL".into(),
        })
    }

    fn stop_recovered(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
    ) -> Result<(), ProcessError> {
        if self.identity_checked(component, record)?.is_none() {
            return Ok(());
        }

        self.host
            .signal_group(record.process_group, SIGTERM)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGTERM failed: {message}"),
            })?;
        if self.wait_for_identity_gone(component, record, self.graceful_stop)? {
            return Ok(());
        }

        // Re-verify immediately before escalation. A reused PID group is not ours.
        if self.identity_checked(component, record)?.is_none() {
            return Ok(());
        }
        self.host
            .signal_group(record.process_group, SIGKILL)
            .map_err(|message| ProcessError::Stop {
                component: component.name(),
                message: format!("SIGKILL failed: {message}"),
            })?;
        if self.wait_for_identity_gone(component, record, self.forced_stop)? {
            return Ok(());
        }
        Err(ProcessError::Stop {
            component: component.name(),
            message: "the process identity remained after SIGKILL".into(),
        })
    }

    fn wait_for_identity_gone(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
        policy: RetryPolicy,
    ) -> Result<bool, ProcessError> {
        self.wait_for_exit(
            || Ok(self.identity_checked(component, record)?.is_none()),
            policy,
        )
    }

    fn identity_checked(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
    ) -> Result<Option<ProcessIdentity>, ProcessError> {
        match self.host.identity(record.pid) {
            Ok(None) => Ok(None),
            Ok(Some(identity)) if record.matches(&identity) => Ok(Some(identity)),
            Ok(Some(_)) => Err(ProcessError::Ambiguous {
                component: component.name(),
                pid: record.pid,
                reason: "the process identity changed before a signal was sent".into(),
            }),
            Err(message) => Err(ProcessError::Ambiguous {
                component: component.name(),
                pid: record.pid,
                reason: format!("the process identity could not be inspected: {message}"),
            }),
        }
    }

    fn wait_for_exit(
        &self,
        mut exited: impl FnMut() -> Result<bool, ProcessError>,
        policy: RetryPolicy,
    ) -> Result<bool, ProcessError> {
        for attempt in 0..policy.attempts {
            if exited()? {
                return Ok(true);
            }
            if let Some(delay) = policy.delay_before_next(attempt) {
                self.waiter.sleep(delay);
            }
        }
        Ok(false)
    }

    fn verify_no_owned_listeners(
        &self,
        component: ProcessComponent,
        record: &OwnedProcessRecord,
    ) -> Result<(), ProcessError> {
        let listeners =
            self.host
                .listeners(component.ports())
                .map_err(|message| ProcessError::Stop {
                    component: component.name(),
                    message: format!("listener inspection failed: {message}"),
                })?;
        let survivors = listeners
            .into_iter()
            .filter(|listener| {
                listener.pid == record.pid || listener.process_group == Some(record.process_group)
            })
            .collect::<Vec<_>>();
        if survivors.is_empty() {
            return Ok(());
        }
        let detail = survivors
            .iter()
            .map(|listener| format!("port {} (pid {})", listener.port, listener.pid))
            .collect::<Vec<_>>()
            .join(", ");
        Err(ProcessError::Stop {
            component: component.name(),
            message: format!("owned listeners remain: {detail}"),
        })
    }

    fn spawn_spec(&self, component: ProcessComponent, configuration: &Path) -> SpawnSpec {
        let configuration = configuration.to_string_lossy().into_owned();
        let (program, arguments) = match component {
            ProcessComponent::CoreDns => (
                canonical_program(&self.paths.coredns_binary),
                vec!["-conf".into(), configuration, "-quiet".into()],
            ),
            ProcessComponent::Caddy => (
                canonical_program(&self.paths.caddy_binary),
                vec!["run".into(), "--config".into(), configuration],
            ),
        };
        let mut environment = vec![
            ("HOME".into(), std::env::var("HOME").unwrap_or_default()),
            (
                "TMPDIR".into(),
                std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()),
            ),
        ];
        if component == ProcessComponent::Caddy {
            environment.push((
                "XDG_DATA_HOME".into(),
                self.paths
                    .caddy_data_directory
                    .to_string_lossy()
                    .into_owned(),
            ));
            environment.push((
                "XDG_CONFIG_HOME".into(),
                self.paths
                    .state_directory
                    .join("caddy-config")
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        SpawnSpec {
            component,
            program,
            arguments,
            working_directory: self.paths.runtime_directory.clone(),
            environment,
            log_path: self.paths.log_directory.join(component.log_name()),
        }
    }
}

pub struct FileProcessRecords {
    directory: PathBuf,
}

impl FileProcessRecords {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            directory: paths.runtime_directory.join("processes"),
        }
    }

    fn path(&self, component: ProcessComponent) -> PathBuf {
        self.directory.join(format!("{}.json", component.key()))
    }
}

impl ProcessRecords for FileProcessRecords {
    fn read(&self, component: ProcessComponent) -> Result<Option<OwnedProcessRecord>, String> {
        let path = self.path(component);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("unable to inspect {}: {error}", path.display())),
        };
        if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
            return Err(format!("process record is invalid: {}", path.display()));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
        let record: OwnedProcessRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("unable to parse {}: {error}", path.display()))?;
        if record.schema_version != RECORD_SCHEMA_VERSION || record.component != component {
            return Err(format!(
                "process record is incompatible: {}",
                path.display()
            ));
        }
        Ok(Some(record))
    }

    fn write(&self, record: &OwnedProcessRecord) -> Result<(), String> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|error| format!("unable to create process records: {error}"))?;
        let path = self.path(record.component);
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| format!("unable to serialize process record: {error}"))?;
        write_atomic(&temporary, &path, &bytes, 0o600)
    }

    fn remove(&self, component: ProcessComponent) -> Result<(), String> {
        let path = self.path(component);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("unable to remove {}: {error}", path.display())),
        }
    }
}

fn write_atomic(
    temporary: &Path,
    destination: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(temporary)
            .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("unable to flush {}: {error}", temporary.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(temporary, bytes)
            .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
    }
    std::fs::rename(temporary, destination).map_err(|error| {
        let _ = std::fs::remove_file(temporary);
        format!("unable to publish {}: {error}", destination.display())
    })
}
