//! Shared on-disk layout pieces for Unix adapters.
//!
//! The install manifest filename is identical on every platform; only the
//! state directory that contains it differs. Broader layout unification
//! waits for a consumer that needs more than the manifest path.

use std::path::{Path, PathBuf};

/// File name of the install manifest inside a state directory.
pub const INSTALL_MANIFEST_FILE_NAME: &str = "install-manifest.json";

/// Resolve `<state_dir>/install-manifest.json`.
pub fn install_manifest_path(state_dir: &Path) -> PathBuf {
    state_dir.join(INSTALL_MANIFEST_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lives_directly_under_the_state_directory() {
        let root = Path::new("/var/lib/own-domains");
        assert_eq!(
            install_manifest_path(root),
            PathBuf::from("/var/lib/own-domains/install-manifest.json")
        );
        assert_eq!(
            install_manifest_path(Path::new("relative/state")),
            PathBuf::from("relative/state/install-manifest.json")
        );
    }
}
