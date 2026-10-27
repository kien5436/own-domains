//! Windows CLI platform: live installer delegation and elevation check.
//!
//! `fltmc` without arguments exits zero only for elevated callers and ships
//! with every Windows install, so no new dependency is needed to gate
//! `--apply` on elevation.

/// Whether the current process runs elevated.
pub(crate) fn is_elevated() -> bool {
    std::process::Command::new("fltmc")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
