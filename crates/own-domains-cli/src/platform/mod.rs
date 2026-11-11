#[cfg(any(
    not(any(target_os = "macos", target_os = "windows", target_os = "linux")),
    test
))]
mod unsupported;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "linux")]
pub(crate) mod linux;

use own_domains_daemon::platform::HostInstaller;
use std::path::{Path, PathBuf};

/// Daemon launch target: `OWN_DOMAINS_DAEMON` when set, else the daemon
/// installed beside this CLI. Installed layouts put both binaries in the
/// same directory (app bundle `Contents/MacOS`, Windows install dir, deb
/// `bin`), so resolving the sibling avoids relying on `PATH`.
pub fn daemon_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("OWN_DOMAINS_DAEMON") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|executable| daemon_beside(&executable))
        .unwrap_or_else(|| PathBuf::from(daemon_name()))
}

fn daemon_name() -> String {
    format!("own-domains-daemon{}", std::env::consts::EXE_SUFFIX)
}

/// Resolve the sibling daemon through symlinks so an installed CLI link
/// into an app bundle finds the daemon in the bundle.
fn daemon_beside(executable: &Path) -> Option<PathBuf> {
    let direct = executable.with_file_name(daemon_name());
    if direct.is_file() {
        return Some(direct);
    }
    // An installed CLI link (macOS app bundle) resolves to its bundle.
    let canonical = executable.canonicalize().ok()?;
    let candidate = canonical.with_file_name(daemon_name());
    candidate.is_file().then_some(candidate)
}

/// Only the Windows installer resolves staged sidecars from the state
/// dir; other platforms ignore it.
#[allow(unused_variables)]
pub fn host_installer(state_dir: &std::path::Path) -> Box<dyn HostInstaller> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsHostInstaller)
    }
    #[cfg(target_os = "windows")]
    {
        let mut installer =
            own_domains_daemon::platform::windows::setup::WindowsHostInstaller::new();
        if let Some(caddy) = windows::staged_caddy_exe(state_dir) {
            installer = installer.with_caddy(caddy);
        }
        Box::new(installer)
    }
    #[cfg(target_os = "linux")]
    {
        let mut installer = own_domains_daemon::platform::linux::setup::LinuxHostInstaller::new();
        if let Some(caddy) = linux::staged_caddy_exe(state_dir) {
            installer = installer.with_caddy(caddy);
        }
        Box::new(installer)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Box::new(unsupported::UnsupportedHostInstaller)
    }
}

#[cfg(test)]
mod tests {
    use super::unsupported;
    use own_domains_daemon::platform::{HostInstaller, SetupRequest, UninstallRequest};
    use std::path::{Path, PathBuf};

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "own-domains-cli-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn daemon_beside_returns_the_sibling_binary() {
        let root = temporary_directory("daemon-sibling");
        let executable = root.join(format!("own-domains{}", std::env::consts::EXE_SUFFIX));
        let daemon = root.join(format!(
            "own-domains-daemon{}",
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::write(&executable, b"cli").unwrap();
        std::fs::write(&daemon, b"daemon").unwrap();

        assert_eq!(
            super::daemon_beside(&executable).map(|path| path.canonicalize().unwrap()),
            Some(daemon.canonicalize().unwrap())
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn daemon_beside_returns_none_without_a_sibling() {
        let root = temporary_directory("daemon-missing");
        let executable = root.join(format!("own-domains{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&executable, b"cli").unwrap();

        assert_eq!(super::daemon_beside(&executable), None);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn daemon_beside_follows_a_symlinked_cli_into_its_bundle() {
        let root = temporary_directory("daemon-symlink");
        let bundle = root.join("app");
        std::fs::create_dir_all(&bundle).unwrap();
        let executable = bundle.join("own-domains");
        let daemon = bundle.join("own-domains-daemon");
        std::fs::write(&executable, b"cli").unwrap();
        std::fs::write(&daemon, b"daemon").unwrap();
        let link = root.join("own-domains");
        std::os::unix::fs::symlink(&executable, &link).unwrap();

        assert_eq!(
            super::daemon_beside(Path::new(&link)),
            Some(daemon.canonicalize().unwrap())
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_installer_returns_actionable_failure() {
        let installer = unsupported::UnsupportedHostInstaller;
        let setup = installer
            .preview_setup(&SetupRequest {
                state_directory: std::path::PathBuf::from("/tmp/state"),
                lan_address: "192.168.1.20".into(),
                interface: "en0".into(),
            })
            .unwrap_err();
        assert!(setup.message.contains("unsupported"));

        let uninstall = installer
            .preview_uninstall(&UninstallRequest {
                state_directory: std::path::PathBuf::from("/tmp/state"),
                purge: false,
            })
            .unwrap_err();
        assert!(uninstall.message.contains("unsupported"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_uses_the_live_installer() {
        let installer = super::host_installer(std::path::Path::new(r"C:\od-state"));
        let preview = installer
            .preview_setup(&SetupRequest {
                state_directory: std::path::PathBuf::from(r"C:\od-state"),
                lan_address: "192.168.1.20".into(),
                interface: "Ethernet".into(),
            })
            .unwrap();
        assert_eq!(preview.actions.len(), 3);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_uses_the_live_installer() {
        let installer = super::host_installer(std::path::Path::new("/tmp/od-state"));
        let preview = installer
            .preview_setup(&SetupRequest {
                state_directory: std::path::PathBuf::from("/tmp/od-state"),
                lan_address: "192.168.1.20".into(),
                interface: "enp3s0".into(),
            })
            .unwrap();
        assert_eq!(preview.actions.len(), 5);
        assert!(
            preview
                .actions
                .iter()
                .all(|action| action.kind.starts_with("linux-"))
        );
    }
}
