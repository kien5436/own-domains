use own_domains_daemon::platform::{
    HostInstaller, SetupApplyResult, SetupFailure, SetupPreview, SetupRequest, UninstallRequest,
};

pub(crate) struct UnsupportedHostInstaller;

impl HostInstaller for UnsupportedHostInstaller {
    fn preview_setup(&self, _request: &SetupRequest) -> Result<SetupPreview, SetupFailure> {
        Err(SetupFailure::new(
            "host setup is unsupported on this platform",
        ))
    }

    fn apply_setup(&self, _request: &SetupRequest) -> Result<SetupApplyResult, SetupFailure> {
        Err(SetupFailure::new(
            "host setup is unsupported on this platform",
        ))
    }

    fn preview_uninstall(&self, _request: &UninstallRequest) -> Result<SetupPreview, SetupFailure> {
        Err(SetupFailure::new(
            "host uninstall is unsupported on this platform",
        ))
    }

    fn apply_uninstall(
        &self,
        _request: &UninstallRequest,
    ) -> Result<SetupApplyResult, SetupFailure> {
        Err(SetupFailure::new(
            "host uninstall is unsupported on this platform",
        ))
    }
}
