//! Windows backend path resolution, tested on every platform.
//!
//! Pure function of its inputs: no process environment, no filesystem writes.
//! All roots derive from the temp dir so they are absolute on every OS.

use own_domains_daemon::platform::windows::backend::resolve_paths;
use std::path::Path;

fn base() -> std::path::PathBuf {
    std::env::temp_dir().join("od-paths-probe")
}

#[test]
fn relative_state_dir_resolves_against_current_dir() {
    let base = base();
    let paths = resolve_paths(Path::new("od-state"), &base, &base.join("exe"), None).unwrap();
    assert_eq!(
        paths.runtime_directory,
        base.join("od-state").join("runtime")
    );
    assert_eq!(paths.coredns_exe, base.join("exe").join("coredns.exe"));
    assert_eq!(paths.caddy_exe, base.join("exe").join("caddy.exe"));
    assert_eq!(
        paths.admin_socket,
        base.join("od-state")
            .join("runtime")
            .join("caddy-admin.sock")
    );
    assert!(paths.admin_socket.is_absolute());
}

#[test]
fn explicit_sidecars_dir_wins_over_exe_dir() {
    let base = base();
    let paths = resolve_paths(
        &base.join("od-state"),
        &base,
        &base.join("exe"),
        Some(base.join("sidecars")),
    )
    .unwrap();
    assert_eq!(
        paths.runtime_directory,
        base.join("od-state").join("runtime")
    );
    assert_eq!(paths.coredns_exe, base.join("sidecars").join("coredns.exe"));
    assert_eq!(paths.caddy_exe, base.join("sidecars").join("caddy.exe"));
}
