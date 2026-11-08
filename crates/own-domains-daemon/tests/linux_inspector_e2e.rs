#![cfg(target_os = "linux")]

//! VM-only: diagnose the real host integration after a privileged setup.
//!
//! Run (as root so `nft` can list the table; `-E` keeps the caller's
//! environment):
//!   sudo -E \
//!     OWN_DOMAINS_TEST_STATE_DIR=<state dir setup used> \
//!     cargo test -p own-domains-daemon --test linux_inspector_e2e
//!
//! The state dir must be the one `setup --apply` wrote (default or the
//! explicit `--state-dir`); setup persists the selected LAN address there,
//! so no LAN env var is needed.

use own_domains_core::{SettingsStore, default_state_dir};
use own_domains_daemon::platform::runtime_backend;
use std::path::PathBuf;

#[test]
fn healthy_host_reports_no_diagnostics() {
    let state = std::env::var_os("OWN_DOMAINS_TEST_STATE_DIR")
        .map(PathBuf::from)
        .or_else(default_state_dir)
        .expect("set OWN_DOMAINS_TEST_STATE_DIR to the state dir setup used (see top of file)");
    let settings = SettingsStore::new(&state)
        .load()
        .expect("setup must have persisted settings in the state dir");
    let backend =
        runtime_backend(&state, None, None, false).expect("the live Linux backend must construct");
    let diagnostics = backend.diagnose(&settings).expect("diagnose must succeed");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}
