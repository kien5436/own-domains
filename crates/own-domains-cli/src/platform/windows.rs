//! Windows CLI platform: live installer delegation and elevation check.
//!
//! `fltmc` without arguments exits zero only for elevated callers and ships
//! with every Windows install, so no new dependency is needed to gate
//! `--apply` on elevation.

use std::path::{Path, PathBuf};

/// Whether the current process runs elevated.
pub(crate) fn is_elevated() -> bool {
    std::process::Command::new("fltmc")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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
    let resources = own_domains_daemon::platform::windows::backend::bundled_resources_dir(exe_dir);
    own_domains_daemon::platform::windows::backend::resolve_paths(
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
