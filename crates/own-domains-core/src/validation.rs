use std::{collections::HashSet, net::Ipv4Addr};

use uuid::Uuid;

use crate::{CoreError, CoreResult, DomainMapping, MappingInput, Settings};

const RESERVED_PORTS: [u32; 4] = [53, 80, 443, 4780];
const RESERVED_HOSTNAMES: [&str; 2] = ["registry.own", "setup.own"];

pub fn normalize_hostname(input: &str) -> String {
    let normalized = input.trim().to_ascii_lowercase();

    normalized.strip_suffix('.').unwrap_or(&normalized).to_string()
}

pub fn validate_hostname(input: &str) -> CoreResult<String> {
    validate_hostname_with_reservations(input, true)
}

pub fn validate_mapping(input: MappingInput) -> CoreResult<DomainMapping> {
    let hostname = validate_hostname(&input.hostname)?;
    if input.upstream_port == 0
        || input.upstream_port > u16::MAX.into()
        || RESERVED_PORTS.contains(&input.upstream_port)
    {
        return validation("Choose an available local port");
    }

    Ok(DomainMapping {
        id: input.id.filter(|id| !id.is_empty()).unwrap_or_else(|| Uuid::new_v4().to_string()),
        hostname,
        upstream_port: input.upstream_port as u16,
        tls: input.tls,
        enabled: input.enabled,
    })
}

pub fn validate_network(lan_address: &str, upstream_dns: &[String]) -> CoreResult<()> {
    let lan = lan_address
        .parse::<Ipv4Addr>()
        .map_err(|_| CoreError::Validation { message: "LAN address must be an IPv4 address".into() })?;
    if upstream_dns.is_empty() {
        return validation("Upstream DNS must contain IPv4 addresses");
    }
    for address in upstream_dns {
        let upstream = address.parse::<Ipv4Addr>().map_err(|_| CoreError::Validation {
            message: "Upstream DNS must contain IPv4 addresses".into(),
        })?;
        if upstream.is_loopback() || upstream == lan {
            return validation("Upstream DNS would create a forwarding loop");
        }
    }
    Ok(())
}

pub fn validate_settings(mut settings: Settings) -> CoreResult<Settings> {
    if settings.schema_version != 1 {
        return validation("unsupported settings schema");
    }
    if !settings.lan_address.is_empty() {
        validate_network(&settings.lan_address, &settings.upstream_dns)?;
    } else if settings.upstream_dns.is_empty()
        || settings.upstream_dns.iter().any(|address| address.parse::<Ipv4Addr>().is_err())
    {
        return validation("Upstream DNS must contain IPv4 addresses");
    }

    let mut ids = HashSet::new();
    let mut hostnames = HashSet::new();
    for mapping in &mut settings.mappings {
        if !ids.insert(mapping.id.clone()) {
            return validation("duplicate mapping identity");
        }
        let hostname = validate_hostname_with_reservations(&mapping.hostname, false)?;
        if !hostnames.insert(hostname.clone()) {
            return Err(CoreError::DuplicateHostname { hostname });
        }
        mapping.hostname = hostname;
        if mapping.upstream_port == 0 || RESERVED_PORTS.contains(&u32::from(mapping.upstream_port)) {
            return validation("Choose an available local port");
        }
    }
    Ok(settings)
}

fn validate_hostname_with_reservations(input: &str, reject_reserved: bool) -> CoreResult<String> {
    let hostname = normalize_hostname(input);
    if hostname.len() > 253 || hostname == "own" || !hostname.ends_with(".own") {
        return validation("Enter a valid .own domain");
    }
    if reject_reserved
        && RESERVED_HOSTNAMES
            .iter()
            .any(|reserved| hostname == *reserved || hostname.ends_with(&format!(".{reserved}")))
    {
        return validation("Reserved .own domain");
    }
    if hostname.split('.').any(|label| !valid_label(label)) {
        return validation("Enter a valid .own domain");
    }
    Ok(hostname)
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label.is_ascii()
        && label.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

fn validation<T>(message: &str) -> CoreResult<T> {
    Err(CoreError::Validation { message: message.into() })
}
