//! Linux host integration (Phase 2).
//!
//! Rendering first: the backend, inspector, and setup transaction build on
//! these constructors. Nothing here touches the machine; privileged setup and
//! uninstall land in a later milestone.

mod config;
mod error;
pub mod interfaces;
pub mod ip;
pub mod nftables;
pub mod systemd;

pub use config::{
    linux_dns_options, linux_proxy_options, render_linux_corefile, render_linux_proxy,
};
pub use error::LinuxArtifactError;
