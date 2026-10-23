#[cfg(any(not(target_os = "macos"), test))]
mod unsupported;

#[cfg(target_os = "macos")]
mod macos;

use own_domains_daemon::platform::HostInstaller;

pub fn host_installer() -> Box<dyn HostInstaller> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsHostInstaller)
    }
    #[cfg(not(target_os = "macos"))]
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
}
