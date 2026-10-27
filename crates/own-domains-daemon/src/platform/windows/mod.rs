//! Windows host integration (Phase 2).
//!
//! Rendering first: the backend, inspector, and setup transaction build on
//! these constructors. Nothing here touches the machine; elevation happens
//! only in setup/uninstall.

pub mod backend;
mod config;
#[cfg(windows)]
pub mod process;

pub use backend::{
    BackendAdmin, BackendChild, BackendProcessHost, BackendReadiness, BackendSetup, SystemSetup,
    WindowsAdapters, WindowsPaths, WindowsRuntimeBackend,
};
pub use config::{windows_dns_options, windows_proxy_options};
