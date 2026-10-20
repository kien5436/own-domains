use own_domains_daemon::{DaemonError, ProcessLock};
use std::{fs, os::unix::fs::PermissionsExt};

fn temporary_directory() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("own-domains-lock-{}", uuid::Uuid::new_v4()))
}

#[test]
fn lock_is_exclusive_and_released_on_drop() {
    let directory = temporary_directory();
    let first = ProcessLock::acquire(&directory).unwrap();
    assert!(matches!(
        ProcessLock::acquire(&directory),
        Err(DaemonError::AlreadyRunning { .. })
    ));
    assert!(ProcessLock::acquire(&temporary_directory()).is_ok());
    drop(first);
    assert!(ProcessLock::acquire(&directory).is_ok());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn state_directory_and_lock_are_owner_only() {
    let directory = temporary_directory();
    let _lock = ProcessLock::acquire(&directory).unwrap();
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(directory.join("own-domains.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::remove_dir_all(directory).unwrap();
}
