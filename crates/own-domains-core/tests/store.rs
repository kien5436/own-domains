use std::{fs, path::PathBuf};

use own_domains_core::{CoreError, Settings, SettingsStore};
use serde_json::json;
use uuid::Uuid;

fn temporary_directory() -> PathBuf {
    std::env::temp_dir().join(format!("own-domains-store-test-{}", Uuid::new_v4()))
}

#[test]
fn missing_file_returns_defaults_and_typescript_json_loads() {
    let directory = temporary_directory();
    let store = SettingsStore::new(&directory);
    assert_eq!(store.load().unwrap(), Settings::default());

    fs::write(
        directory.join("settings.json"),
        r#"{"schemaVersion":1,"revision":4,"lanAddress":"192.168.1.20","upstreamDns":["1.1.1.1"],"mappings":[]}"#,
    )
    .unwrap();
    assert_eq!(store.load().unwrap().revision, 4);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn corrupt_or_invalid_settings_are_preserved() {
    for input in [
        "{not json}",
        r#"{"schemaVersion":2,"revision":0,"lanAddress":"","upstreamDns":["1.1.1.1"],"mappings":[]}"#,
        r#"{"schemaVersion":1,"revision":0,"lanAddress":"","upstreamDns":["1.1.1.1"],"mappings":[{"id":"same","hostname":"one.own","upstreamPort":3000,"tls":false,"enabled":true},{"id":"same","hostname":"two.own","upstreamPort":3001,"tls":false,"enabled":true}]}"#,
    ] {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        fs::write(&path, input).unwrap();

        assert!(matches!(SettingsStore::new(&directory).load(), Err(CoreError::CorruptSettings { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), input);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn unknown_top_level_fields_survive_load_and_save() {
    let directory = temporary_directory();
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("settings.json"),
        r#"{"schemaVersion":1,"revision":1,"lanAddress":"","upstreamDns":["1.1.1.1"],"mappings":[],"futureFlag":{"enabled":true}}"#,
    )
    .unwrap();
    let store = SettingsStore::new(&directory);
    let mut settings = store.load().unwrap();
    assert_eq!(settings.extra.get("futureFlag"), Some(&json!({"enabled": true})));
    settings.revision = 2;
    store.save(&settings, Some(1)).unwrap();
    assert_eq!(store.load().unwrap().extra.get("futureFlag"), Some(&json!({"enabled": true})));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn save_is_revision_checked_and_newline_terminated() {
    let directory = temporary_directory();
    let store = SettingsStore::new(&directory);
    let mut settings = Settings { revision: 3, ..Settings::default() };
    store.save(&settings, Some(0)).unwrap();
    let original = fs::read(directory.join("settings.json")).unwrap();
    assert_eq!(original.last(), Some(&b'\n'));

    settings.revision = 4;
    assert!(matches!(store.save(&settings, Some(2)), Err(CoreError::RevisionConflict { expected: 2, actual: 3 })));
    assert_eq!(fs::read(directory.join("settings.json")).unwrap(), original);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_temporary_write_preserves_previous_settings() {
    let directory = temporary_directory();
    let store = SettingsStore::new(&directory);
    let original = Settings { revision: 1, ..Settings::default() };
    store.save(&original, Some(0)).unwrap();
    fs::create_dir(directory.join("settings.json.tmp")).unwrap();

    let updated = Settings { revision: 2, ..Settings::default() };
    assert!(matches!(store.save(&updated, Some(1)), Err(CoreError::Io { .. })));
    assert_eq!(store.load().unwrap().revision, 1);
    assert!(directory.join("settings.json.tmp").exists());
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn settings_directory_and_file_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary_directory();
    let store = SettingsStore::new(&directory);
    store.save(&Settings::default(), Some(0)).unwrap();

    assert_eq!(fs::metadata(&directory).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(directory.join("settings.json")).unwrap().permissions().mode() & 0o777, 0o600);
    fs::remove_dir_all(directory).unwrap();
}
