use std::net::Ipv4Addr;

use crate::{CoreError, CoreResult, Settings, validate_network};

pub struct DnsConfigOptions {
    pub listen_addresses: Vec<Ipv4Addr>,
    pub port: u16,
    pub upstream_port: u16,
}

impl DnsConfigOptions {
    pub fn for_settings(settings: &Settings) -> CoreResult<Self> {
        validate_network(&settings.lan_address, &settings.upstream_dns)?;
        let lan = settings
            .lan_address
            .parse()
            .map_err(|_| CoreError::Validation {
                message: "A valid LAN IPv4 address is required".into(),
            })?;
        Ok(Self {
            listen_addresses: vec![Ipv4Addr::LOCALHOST, lan],
            port: 5354,
            upstream_port: 53,
        })
    }
}

pub fn build_corefile(settings: &Settings, options: &DnsConfigOptions) -> CoreResult<String> {
    validate_network(&settings.lan_address, &settings.upstream_dns)?;
    let lan: Ipv4Addr = settings
        .lan_address
        .parse()
        .map_err(|_| CoreError::Validation {
            message: "A valid LAN IPv4 address is required".into(),
        })?;
    if options.listen_addresses.is_empty()
        || options
            .listen_addresses
            .iter()
            .any(|address| *address != Ipv4Addr::LOCALHOST && *address != lan)
    {
        return Err(CoreError::Validation {
            message: "DNS listeners must use loopback or the selected LAN address".into(),
        });
    }
    let listeners = options
        .listen_addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let upstreams = settings
        .upstream_dns
        .iter()
        .map(|address| format!("{address}:{}", options.upstream_port))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        "own:{port} {{\n    bind {listeners}\n    template IN A {{\n        match ^(?:[a-z0-9-]+\\.)+own\\.$\n        answer \"{{{{ .Name }}}} 60 IN A {lan}\"\n    }}\n    template IN AAAA {{\n        rcode NOERROR\n    }}\n}}\n.:{port} {{\n    bind {listeners}\n    forward . {upstreams}\n}}\n",
        port = options.port
    ))
}
