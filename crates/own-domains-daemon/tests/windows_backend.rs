#![cfg(target_os = "windows")]

//! Windows process supervision primitives (no Job Objects yet).

use own_domains_daemon::platform::windows::process::{WindowsChild, any_visible_top_window};
use std::time::Duration;

#[test]
fn short_lived_child_exits_with_its_status() {
    let child = WindowsChild::spawn_no_window("cmd /c exit 42").unwrap();
    assert!(child.pid() > 0);
    assert_eq!(child.wait_timeout(Duration::from_secs(10)).unwrap(), 42);
}

#[test]
fn sleeper_has_no_console_window_and_terminates() {
    let child = WindowsChild::spawn_no_window("ping -n 30 127.0.0.1 >NUL").unwrap();
    assert!(!any_visible_top_window(child.pid()));
    assert!(child.try_wait().unwrap().is_none());
    child.terminate().unwrap();
    let status = child.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_ne!(
        status, 259,
        "TerminateProcess exit code must not be STILL_ACTIVE"
    );
}
