//! Windows host integration (Phase 2).
//!
//! Rendering first: the backend, inspector, and setup transaction build on
//! these constructors. Nothing here touches the machine; elevation happens
//! only in setup/uninstall.

pub mod backend;
mod config;
#[cfg(target_os = "windows")]
pub(crate) mod factory;
#[cfg(windows)]
pub mod process;

#[cfg(target_os = "windows")]
pub(crate) use factory::runtime_backend;

#[cfg(target_os = "windows")]
pub use backend::SystemProcessHost;
pub use backend::{
    BackendAdmin, BackendChild, BackendProcessHost, BackendReadiness, BackendSetup,
    SystemReadiness, SystemSetup, WindowsAdapters, WindowsPaths, WindowsRuntimeBackend,
    resolve_paths,
};
pub use config::{windows_dns_options, windows_proxy_options};
