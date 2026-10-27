//! Facade selection on Windows: settings-only yields the unavailable backend,
//! otherwise the live Windows backend. Runs on the Windows guest only.

#![cfg(target_os = "windows")]

use own_domains_core::Settings;
use own_domains_daemon::RuntimeError;

fn temp_state_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("od-facade-{tag}-{}", std::process::id()))
}

#[test]
fn settings_only_yields_unavailable_backend() {
    let backend = own_domains_daemon::platform::runtime_backend(
        &temp_state_dir("settings"),
        None,
        None,
        true,
    )
    .unwrap();
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Stopped
    ));
    assert!(matches!(
        backend.apply(&Settings::default()),
        Err(RuntimeError::Unavailable(_))
    ));
    assert!(backend.diagnose(&Settings::default()).unwrap().is_empty());
}

#[test]
fn live_backend_reports_missing_sidecars() {
    let state = temp_state_dir("live");
    let backend = own_domains_daemon::platform::runtime_backend(
        &state,
        None,
        Some(state.join("sidecars")),
        false,
    )
    .unwrap();
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Stopped
    ));
    // Proof this is the live backend: the unavailable one reports nothing.
    let diagnostics = backend.diagnose(&Settings::default()).unwrap();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "missingSidecar"),
        "{diagnostics:?}"
    );
}
