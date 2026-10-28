//! Windows host artifact rendering.
//!
//! CoreDNS owns port 53 (NRPT has no custom-port parameter) and serves TTL 0;
//! Caddy binds 80/443 directly with HTTP/3 disabled so it never opens a UDP
//! 443 listener. Listeners are loopback plus the selected LAN address only.

use std::net::Ipv4Addr;

use own_domains_core::{CoreError, CoreResult, DnsConfigOptions, ProxyConfigOptions, Settings};

/// Loopback-only Caddy admin endpoint. Caddy on Windows cannot serve its
/// admin API over a Unix socket (the process dies during startup with no log
/// or exit code), so the admin listener is loopback TCP by plan. Binding
/// 127.0.0.1 keeps it unreachable from LAN interfaces by construction.
pub const CADDY_ADMIN_PORT: u16 = 2019;

pub fn windows_dns_options(settings: &Settings) -> CoreResult<DnsConfigOptions> {
    let lan = lan_address(settings)?;
    Ok(DnsConfigOptions {
        listen_addresses: vec![Ipv4Addr::LOCALHOST, lan],
        port: 53,
        upstream_port: 53,
        ttl_seconds: 0,
    })
}

pub fn windows_proxy_options(
    settings: &Settings,
    admin_port: u16,
) -> CoreResult<ProxyConfigOptions> {
    let lan = lan_address(settings)?;
    Ok(ProxyConfigOptions {
        admin_listen: format!("127.0.0.1:{admin_port}"),
        listen_addresses: vec![Ipv4Addr::LOCALHOST, lan],
        http_port: 80,
        https_port: 443,
        setup_port: 4781,
        protocols: Some(vec!["h1".to_string(), "h2".to_string()]),
    })
}

fn lan_address(settings: &Settings) -> CoreResult<Ipv4Addr> {
    settings
        .lan_address
        .parse()
        .map_err(|_| CoreError::Validation {
            message: "A valid LAN IPv4 address is required".into(),
        })
}
