#[cfg(any(not(any(target_os = "macos", target_os = "windows")), test))]
mod unsupported;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
pub(crate) mod windows;

use own_domains_daemon::platform::HostInstaller;

pub fn host_installer() -> Box<dyn HostInstaller> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsHostInstaller)
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(own_domains_daemon::platform::windows::setup::WindowsHostInstaller::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
        let installer = super::host_installer();
        let preview = installer
            .preview_setup(&SetupRequest {
                state_directory: std::path::PathBuf::from(r"C:\od-state"),
                lan_address: "192.168.1.20".into(),
                interface: "Ethernet".into(),
            })
            .unwrap();
        assert_eq!(preview.actions.len(), 3);
    }
}
