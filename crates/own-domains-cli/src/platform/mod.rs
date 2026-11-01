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
