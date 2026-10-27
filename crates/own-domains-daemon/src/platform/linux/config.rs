//! Linux host artifact rendering.
//!
//! Linux serves TTL 0 so client caches do not hide address changes, and
//! advertises h1/h2 so Caddy never opens a UDP 443 listener. Listeners are
//! loopback plus the selected LAN address only.

use std::path::Path;

use own_domains_core::{
    CoreResult, DnsConfigOptions, ProxyConfigOptions, Settings, build_corefile, build_proxy_config,
};
use serde_json::Value;

pub fn linux_dns_options(settings: &Settings) -> CoreResult<DnsConfigOptions> {
    let mut options = DnsConfigOptions::for_settings(settings)?;
    options.ttl_seconds = 0;
    Ok(options)
}

pub fn linux_proxy_options(
    settings: &Settings,
    admin_socket: &Path,
) -> CoreResult<ProxyConfigOptions> {
    let mut options = ProxyConfigOptions::for_runtime(settings, admin_socket)?;
    options.protocols = Some(vec!["h1".into(), "h2".into()]);
    Ok(options)
}

pub fn render_linux_corefile(settings: &Settings) -> CoreResult<String> {
    build_corefile(settings, &linux_dns_options(settings)?)
}

pub fn render_linux_proxy(settings: &Settings, admin_socket: &Path) -> CoreResult<Value> {
    build_proxy_config(settings, &linux_proxy_options(settings, admin_socket)?)
}
