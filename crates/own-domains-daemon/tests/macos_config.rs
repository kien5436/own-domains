#![cfg(target_os = "macos")]
use own_domains_core::{DomainMapping, Settings};
use own_domains_daemon::platform::macos::{
    ConfigurationError, ConfigurationFiles, ConfigurationStager, ConfigurationValidator,
    HostCommand, MacOsArchitecture, RuntimePaths,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

#[derive(Default)]
struct FakeFiles {
    state: Mutex<FilesState>,
}

#[derive(Default)]
struct FilesState {
    files: BTreeMap<PathBuf, (Vec<u8>, u32)>,
    fail_write: Option<PathBuf>,
}

impl FakeFiles {
    fn fail_next_write(&self, path: PathBuf) {
        self.state.lock().unwrap().fail_write = Some(path);
    }

    fn text(&self, path: &Path) -> String {
        String::from_utf8(self.bytes(path)).unwrap()
    }

    fn bytes(&self, path: &Path) -> Vec<u8> {
        self.state
            .lock()
            .unwrap()
            .files
            .get(path)
            .map(|(contents, _)| contents.clone())
            .unwrap_or_else(|| panic!("missing file {}", path.display()))
    }

    fn optional_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .files
            .get(path)
            .map(|(contents, _)| contents.clone())
    }

    fn mode(&self, path: &Path) -> u32 {
        self.state
            .lock()
            .unwrap()
            .files
            .get(path)
            .map(|(_, mode)| *mode)
            .unwrap_or_else(|| panic!("missing file {}", path.display()))
    }
}

impl ConfigurationFiles for FakeFiles {
    fn create_dir_all(&self, _path: &Path) -> Result<(), String> {
        Ok(())
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        Ok(self.optional_bytes(path))
    }

    fn write_atomic(&self, path: &Path, contents: &[u8], mode: u32) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.fail_write.as_deref() == Some(path) {
            state.fail_write = None;
            return Err(format!("injected write failure at {}", path.display()));
        }
        state
            .files
            .insert(path.to_path_buf(), (contents.to_vec(), mode));
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let file = state
            .files
            .remove(from)
            .ok_or_else(|| format!("missing rename source {}", from.display()))?;
        state.files.insert(to.to_path_buf(), file);
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        self.state.lock().unwrap().files.remove(path);
        Ok(())
    }

    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .files
            .retain(|candidate, _| !candidate.starts_with(path));
        Ok(())
    }
}

#[derive(Default)]
struct FakeValidator {
    state: Mutex<ValidatorState>,
}

#[derive(Default)]
struct ValidatorState {
    terminating: Vec<HostCommand>,
    starting: Vec<(HostCommand, Duration)>,
    fail_terminating: bool,
    fail_starting: bool,
}

impl FakeValidator {
    fn fail_terminating(&self) {
        self.state.lock().unwrap().fail_terminating = true;
    }

    fn terminating(&self) -> Vec<HostCommand> {
        self.state.lock().unwrap().terminating.clone()
    }

    fn starting(&self) -> Vec<(HostCommand, Duration)> {
        self.state.lock().unwrap().starting.clone()
    }
}

impl ConfigurationValidator for FakeValidator {
    fn run(&self, command: &HostCommand) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.terminating.push(command.clone());
        if state.fail_terminating {
            return Err("injected validation failure".into());
        }
        Ok(())
    }

    fn expect_running(&self, command: &HostCommand, settle: Duration) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.starting.push((command.clone(), settle));
        if state.fail_starting {
            return Err("injected startup failure".into());
        }
        Ok(())
    }
}

fn paths() -> RuntimePaths {
    RuntimePaths::new(
        PathBuf::from("/Users/kien/Library/Application Support/OwnDomains"),
        PathBuf::from("/Applications/Own Domains.app/Contents/Resources"),
        PathBuf::from("/Users/kien/Library/Application Support/OwnDomains/backups/setup-1"),
        MacOsArchitecture::Arm64,
    )
    .unwrap()
}

fn settings(revision: u64) -> Settings {
    Settings {
        revision,
        lan_address: "192.168.1.20".into(),
        mappings: vec![DomainMapping {
            id: "shop".into(),
            hostname: "shop.own".into(),
            upstream_port: 3000,
            tls: false,
            enabled: true,
        }],
        ..Settings::default()
    }
}

fn applied_snapshot(files: &FakeFiles, paths: &RuntimePaths) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    [
        paths.applied_directory.join("Corefile"),
        paths.applied_directory.join("caddy.json"),
        paths.applied_directory.join("revision"),
    ]
    .into_iter()
    .map(|path| {
        let contents = files.optional_bytes(&path);
        (path, contents)
    })
    .collect()
}

#[test]
fn stages_owner_only_candidates_and_validates_with_fixed_commands() {
    let paths = paths();
    let files = FakeFiles::default();
    let validator = FakeValidator::default();
    let stager = ConfigurationStager::new(&paths, &files, &validator);

    let candidate = stager.stage(&settings(4)).unwrap();

    assert_eq!(candidate.revision, 4);
    assert_eq!(
        candidate.corefile_path,
        paths.candidate_directory.join("Corefile")
    );
    assert_eq!(
        candidate.proxy_path,
        paths.candidate_directory.join("caddy.json")
    );
    assert_eq!(
        candidate.validation_corefile_path,
        paths.candidate_directory.join("Corefile.validation")
    );

    let corefile = files.text(&candidate.corefile_path);
    assert!(corefile.starts_with("own:5354 {"));
    assert!(!corefile.contains(".own:"));
    assert!(corefile.contains("bind 127.0.0.1 192.168.1.20"));
    assert!(corefile.contains("forward . 1.1.1.1:53 9.9.9.9:53"));

    let validation = files.text(&candidate.validation_corefile_path);
    assert!(validation.starts_with("own:15354 {"));
    assert!(validation.contains(".:15354"));
    assert!(validation.contains("bind 127.0.0.1\n"));
    assert!(!validation.contains("bind 127.0.0.1 192.168.1.20"));
    assert!(!validation.contains(":5354"));

    let proxy: Value =
        serde_json::from_slice(&files.bytes(&candidate.proxy_path)).expect("valid caddy JSON");
    assert_eq!(
        proxy["admin"]["listen"],
        format!(
            "unix/{}",
            paths.runtime_directory.join("caddy-admin.sock").display()
        )
    );
    assert_eq!(proxy["apps"]["http"]["http_port"], 8080);
    assert_eq!(proxy["apps"]["http"]["https_port"], 8443);
    assert_eq!(proxy["storage"]["module"], "file_system");
    assert_eq!(
        proxy["storage"]["root"],
        paths.caddy_data_directory.to_string_lossy().as_ref()
    );

    for path in [
        &candidate.corefile_path,
        &candidate.proxy_path,
        &candidate.validation_corefile_path,
        &paths.candidate_directory.join("revision"),
    ] {
        assert_eq!(
            files.mode(path),
            0o600,
            "{} must be owner-only",
            path.display()
        );
    }

    stager.validate(&candidate).unwrap();

    let caddy = validator.terminating();
    assert_eq!(caddy.len(), 1);
    assert_eq!(caddy[0].program, paths.caddy_binary);
    assert_eq!(
        caddy[0].args,
        vec![
            "validate".to_string(),
            "--config".to_string(),
            candidate.proxy_path.to_string_lossy().into_owned(),
        ]
    );

    let coredns = validator.starting();
    assert_eq!(coredns.len(), 1);
    assert_eq!(coredns[0].0.program, paths.coredns_binary);
    assert_eq!(
        coredns[0].0.args,
        vec![
            "-conf".to_string(),
            candidate
                .validation_corefile_path
                .to_string_lossy()
                .into_owned(),
        ]
    );
    assert!(coredns[0].1 >= Duration::from_millis(100));
}

#[test]
fn validation_failure_reports_the_component_and_keeps_candidates_for_diagnosis() {
    let paths = paths();
    let files = FakeFiles::default();
    let validator = FakeValidator::default();
    let stager = ConfigurationStager::new(&paths, &files, &validator);
    let candidate = stager.stage(&settings(5)).unwrap();

    validator.fail_terminating();

    let error = stager.validate(&candidate).unwrap_err();
    assert!(
        matches!(&error, ConfigurationError::ValidationFailure(message) if message.contains("Caddy"))
    );
    assert!(files.optional_bytes(&candidate.corefile_path).is_some());
    assert!(files.optional_bytes(&candidate.proxy_path).is_some());
    assert_eq!(validator.starting().len(), 0);
}

#[test]
fn failed_promotion_restores_the_previous_applied_configuration_and_revision() {
    let paths = paths();
    let files = FakeFiles::default();
    let validator = FakeValidator::default();
    let stager = ConfigurationStager::new(&paths, &files, &validator);

    let first = stager.stage(&settings(7)).unwrap();
    stager.validate(&first).unwrap();
    assert_eq!(stager.promote(&first).unwrap(), 7);
    let before = applied_snapshot(&files, &paths);

    let second = stager.stage(&settings(8)).unwrap();
    files.fail_next_write(paths.applied_directory.join("caddy.json"));

    let error = stager.promote(&second).unwrap_err();
    assert!(matches!(error, ConfigurationError::Promotion(_)));
    assert_eq!(applied_snapshot(&files, &paths), before);
    assert!(files.optional_bytes(&second.corefile_path).is_some());
}

#[test]
fn promotion_records_previous_and_applied_revisions_and_restore_reverts_them() {
    let paths = paths();
    let files = FakeFiles::default();
    let validator = FakeValidator::default();
    let stager = ConfigurationStager::new(&paths, &files, &validator);

    let first = stager.stage(&settings(1)).unwrap();
    stager.validate(&first).unwrap();
    stager.promote(&first).unwrap();
    let first_corefile = files.bytes(&paths.applied_directory.join("Corefile"));

    let second_settings = Settings {
        lan_address: "192.168.1.21".into(),
        ..settings(2)
    };
    let second = stager.stage(&second_settings).unwrap();
    stager.validate(&second).unwrap();
    assert_eq!(stager.promote(&second).unwrap(), 2);
    assert_eq!(files.text(&paths.applied_directory.join("revision")), "2");
    assert_ne!(
        files.bytes(&paths.applied_directory.join("Corefile")),
        first_corefile
    );
    assert_eq!(
        files.text(&paths.runtime_directory.join("previous/revision")),
        "1"
    );

    assert_eq!(stager.restore().unwrap(), Some(1));
    assert_eq!(files.text(&paths.applied_directory.join("revision")), "1");
    assert_eq!(
        files.bytes(&paths.applied_directory.join("Corefile")),
        first_corefile
    );
    assert_eq!(stager.restore().unwrap(), None);
}

#[test]
fn rejects_a_validation_port_that_conflicts_with_runtime_ports() {
    let paths = paths();
    let files = FakeFiles::default();
    let validator = FakeValidator::default();

    let stager = ConfigurationStager::new(&paths, &files, &validator).with_validation_port(5354);
    let error = stager.stage(&settings(1)).unwrap_err();
    assert!(matches!(error, ConfigurationError::Validation(_)));
}
