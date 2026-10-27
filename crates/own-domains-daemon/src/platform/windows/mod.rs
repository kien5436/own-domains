//! Windows host integration (Phase 2).
//!
//! Rendering first: the backend, inspector, and setup transaction build on
//! these constructors. Nothing here touches the machine; elevation happens
//! only in setup/uninstall.

mod config;
pub mod process;

pub use config::{windows_dns_options, windows_proxy_options};
