//! Platform default locations for the single-user state directory.
//!
//! The GUI and CLI share this default so a bare launch on any host uses
//! the same registry. Explicit `--state-dir` (or `OWN_DOMAINS_STATE_DIR`
//! for the desktop) always wins.

use std::path::PathBuf;

/// Absolute default state directory, or `None` when the base location
/// cannot be determined on this platform.
pub fn default_state_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("OwnDomains"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(PathBuf::from).map(|base| {
            base.join("Library")
                .join("Application Support")
                .join("OwnDomains")
        })
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("state"))
            })
            .map(|base| base.join("own-domains"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::default_state_dir;

    #[test]
    fn default_state_dir_is_absolute_when_defined() {
        let Some(dir) = default_state_dir() else {
            return;
        };
        assert!(dir.is_absolute(), "{dir:?}");
    }
}
