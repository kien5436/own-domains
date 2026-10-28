//! Windows backend path resolution, tested on every platform.
//!
//! Pure function of its inputs: no process environment, no filesystem writes.
//! All roots derive from the temp dir so they are absolute on every OS.

use own_domains_daemon::platform::windows::{CADDY_ADMIN_PORT, backend::resolve_paths};
use std::path::Path;

fn base() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("od-paths-probe-{}", uuid::Uuid::new_v4()))
}

#[test]
fn relative_state_dir_resolves_against_current_dir() {
    let base = base();
    let paths = resolve_paths(Path::new("od-state"), &base, &base.join("exe"), None, None).unwrap();
    assert_eq!(
        paths.runtime_directory,
        base.join("od-state").join("runtime")
    );
    assert_eq!(paths.coredns_exe, base.join("exe").join("coredns.exe"));
    assert_eq!(paths.caddy_exe, base.join("exe").join("caddy.exe"));
    assert_eq!(paths.admin_port, CADDY_ADMIN_PORT);
    assert_eq!(
        paths.caddy_data_directory,
        base.join("od-state").join("caddy-data")
    );
}

#[test]
#[cfg(target_os = "windows")]
fn bundled_sidecars_win_over_exe_dir() {
    let base = base();
    let triple = bundled_triple();
    let bundled = base.join("resources").join("sidecars").join(triple);
    std::fs::create_dir_all(&bundled).unwrap();
    std::fs::write(bundled.join(format!("coredns-{triple}.exe")), b"fake").unwrap();
    std::fs::write(bundled.join(format!("caddy-{triple}.exe")), b"fake").unwrap();
    let paths = resolve_paths(
        &base.join("od-state"),
        &base,
        &base.join("exe"),
        Some(&base.join("resources")),
        None,
    )
    .unwrap();
    assert_eq!(
        paths.coredns_exe,
        bundled.join(format!("coredns-{triple}.exe"))
    );
    assert_eq!(paths.caddy_exe, bundled.join(format!("caddy-{triple}.exe")));
}

#[test]
fn explicit_sidecars_dir_wins_over_bundle() {
    let base = base();
    let triple = bundled_triple();
    let bundled = base.join("resources").join("sidecars").join(triple);
    std::fs::create_dir_all(&bundled).unwrap();
    std::fs::write(bundled.join(format!("coredns-{triple}.exe")), b"fake").unwrap();
    std::fs::write(bundled.join(format!("caddy-{triple}.exe")), b"fake").unwrap();
    let paths = resolve_paths(
        &base.join("od-state"),
        &base,
        &base.join("exe"),
        Some(&base.join("resources")),
        Some(base.join("sidecars")),
    )
    .unwrap();
    assert_eq!(paths.coredns_exe, base.join("sidecars").join("coredns.exe"));
}

#[test]
fn partial_bundle_falls_back_to_exe_dir() {
    let base = base();
    let triple = bundled_triple();
    let bundled = base.join("resources").join("sidecars").join(triple);
    std::fs::create_dir_all(&bundled).unwrap();
    std::fs::write(bundled.join(format!("coredns-{triple}.exe")), b"fake").unwrap();
    let paths = resolve_paths(
        &base.join("od-state"),
        &base,
        &base.join("exe"),
        Some(&base.join("resources")),
        None,
    )
    .unwrap();
    assert_eq!(paths.coredns_exe, base.join("exe").join("coredns.exe"));
}

/// The Windows target triple for this build. On non-Windows hosts it is only
/// a directory label; the bundled-layout tests run everywhere.
fn bundled_triple() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64-pc-windows-msvc"
    } else {
        "aarch64-pc-windows-msvc"
    }
}

#[test]
fn explicit_sidecars_dir_wins_over_exe_dir() {
    let base = base();
    let paths = resolve_paths(
        &base.join("od-state"),
        &base,
        &base.join("exe"),
        None,
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
