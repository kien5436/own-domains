//! Windows host artifact rendering.
//!
//! CoreDNS owns port 53 (NRPT has no custom-port parameter) and serves TTL 0;
//! Caddy binds 80/443 directly with HTTP/3 disabled so it never opens a UDP
//! 443 listener. Listeners are loopback plus the selected LAN address only.

use std::{net::Ipv4Addr, path::Path};

use own_domains_core::{CoreError, CoreResult, DnsConfigOptions, ProxyConfigOptions, Settings};

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
    admin_socket: &Path,
) -> CoreResult<ProxyConfigOptions> {
    if !admin_socket.is_absolute() {
        return Err(CoreError::Validation {
            message: "Caddy admin socket must be absolute".into(),
        });
    }
    let lan = lan_address(settings)?;
    Ok(ProxyConfigOptions {
        admin_listen: format!("unix/{}", admin_socket.display()),
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
