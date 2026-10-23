use own_domains_core::{Diagnostic, Settings};
use own_domains_daemon::platform::{
    HostInspector, HostInstaller, SetupApplyResult, SetupFailure, SetupPreview, SetupRequest,
    UninstallRequest,
};

struct FakeInstaller;

impl HostInstaller for FakeInstaller {
    fn preview_setup(&self, _request: &SetupRequest) -> Result<SetupPreview, SetupFailure> {
        Ok(SetupPreview {
            actions: Vec::new(),
        })
    }

    fn apply_setup(&self, _request: &SetupRequest) -> Result<SetupApplyResult, SetupFailure> {
        Ok(SetupApplyResult {
            written: Vec::new(),
            unchanged: Vec::new(),
            backups: Vec::new(),
        })
    }

    fn preview_uninstall(&self, _request: &UninstallRequest) -> Result<SetupPreview, SetupFailure> {
        Ok(SetupPreview {
            actions: Vec::new(),
        })
    }

    fn apply_uninstall(
        &self,
        _request: &UninstallRequest,
    ) -> Result<SetupApplyResult, SetupFailure> {
        Ok(SetupApplyResult {
            written: Vec::new(),
            unchanged: Vec::new(),
            backups: Vec::new(),
        })
    }
}

struct FakeInspector;

impl HostInspector for FakeInspector {
    fn inspect(&self, _settings: &Settings) -> Vec<Diagnostic> {
        Vec::new()
    }
}

fn assert_send_sync<T: Send + Sync + ?Sized>() {}

#[test]
fn shared_host_contracts_are_object_safe_and_send_sync() {
    assert_send_sync::<dyn HostInstaller>();
    assert_send_sync::<dyn HostInspector>();
    let _installer: Box<dyn HostInstaller> = Box::new(FakeInstaller);
    let _inspector: Box<dyn HostInspector> = Box::new(FakeInspector);
}

#[test]
fn setup_failures_preserve_their_cli_exit_code() {
    assert_eq!(SetupFailure::new("setup failed").exit_code, 4);
    assert_eq!(
        SetupFailure::new("invalid input")
            .with_exit_code(2)
            .exit_code,
        2
    );
}
