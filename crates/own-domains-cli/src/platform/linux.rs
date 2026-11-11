//! Linux CLI platform: live installer delegation and root gating.
//!
//! `sudo` is the only privileged path: applying setup requires euid 0, and the
//! installer resolves the target user from `SUDO_USER`/`SUDO_UID` itself.
//! SSH sessions are fine — root writes system paths either way.

use std::path::{Path, PathBuf};

/// Whether the current process runs as root.
pub(crate) fn is_root() -> bool {
    let euid = unsafe { libc::geteuid() };
    euid == 0
}

/// Staged Caddy for setup-time CA generation, resolved the same way the
/// daemon resolves its bundle: beside the `OWN_DOMAINS_DAEMON` executable
/// (or the CLI sibling of the same name), under an installed
/// `sidecars/<triple>/` layout or as exe-adjacent plain names. `None`
/// when nothing usable is staged; setup then fails with an actionable
/// message instead of guessing.
pub(crate) fn staged_caddy_exe(state_dir: &Path) -> Option<PathBuf> {
    let daemon = std::env::var_os("OWN_DOMAINS_DAEMON")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::current_exe().ok().map(|exe| {
                exe.with_file_name(format!(
                    "own-domains-daemon{}",
                    std::env::consts::EXE_SUFFIX
                ))
            })
        })?;
    let exe_dir = daemon.parent()?;
    let current_dir = std::env::current_dir().ok()?;
    let resources = own_domains_daemon::platform::linux::backend::bundled_resources_dir(exe_dir);
    own_domains_daemon::platform::linux::backend::resolve_paths(
        state_dir,
        &current_dir,
        exe_dir,
        resources.as_deref(),
        None,
    )
    .ok()
    .map(|paths| paths.caddy_exe)
    .filter(|exe| exe.is_file())
}
