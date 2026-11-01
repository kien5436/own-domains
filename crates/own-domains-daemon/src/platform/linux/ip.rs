//! `ip -j` discovery and primary address selection.
//!
//! Parses the `/usr/sbin/ip -j link show` and `/usr/sbin/ip -j -4 addr show`
//! shapes. Links with a `master` (slaves) and virtual link types never
//! qualify; of the remaining eligible names, the primary address is the
//! first `inet` global non-secondary IPv4 address that is not loopback,
//! link-local, multicast, or unspecified.

use std::{collections::HashMap, net::Ipv4Addr};

use serde::Deserialize;

use super::{error::LinuxArtifactError, interfaces::is_eligible_interface_name};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinuxInterface {
    pub name: String,
    pub primary_ipv4: Option<Ipv4Addr>,
}

#[derive(Debug, Deserialize)]
struct LinkEntry {
    ifname: String,
    #[serde(default)]
    master: Option<String>,
    #[serde(default)]
    link_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AddrEntry {
    ifname: String,
    #[serde(default)]
    addr_info: Vec<AddrInfo>,
}

#[derive(Debug, Deserialize)]
struct AddrInfo {
    #[serde(default)]
    family: String,
    #[serde(default)]
    local: Option<String>,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    secondary: bool,
}

const EXCLUDED_LINK_TYPES: &[&str] = &["loopback", "dummy", "veth", "tun", "wireguard"];

fn parse_json<T>(value: &str) -> Result<T, LinuxArtifactError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(value)
        .map_err(|error| LinuxArtifactError::InvalidIpOutput(error.to_string()))
}

fn primary_address(infos: &[AddrInfo]) -> Option<Ipv4Addr> {
    infos.iter().find_map(|info| {
        if info.family != "inet" || info.scope != "global" || info.secondary {
            return None;
        }
        let address: Ipv4Addr = info.local.as_deref()?.parse().ok()?;
        if address.is_loopback()
            || address.is_link_local()
            || address.is_multicast()
            || address.is_unspecified()
        {
            return None;
        }
        Some(address)
    })
}

pub fn discover_covered_interfaces(
    link_json: &str,
    addr_json: &str,
) -> Result<Vec<LinuxInterface>, LinuxArtifactError> {
    let links: Vec<LinkEntry> = parse_json(link_json)?;
    let addresses: Vec<AddrEntry> = parse_json(addr_json)?;
    let by_name: HashMap<&str, &[AddrInfo]> = addresses
        .iter()
        .map(|entry| (entry.ifname.as_str(), entry.addr_info.as_slice()))
        .collect();

    let mut covered = Vec::new();
    for link in &links {
        if link.master.is_some() {
            continue;
        }
        if link
            .link_type
            .as_deref()
            .is_some_and(|kind| EXCLUDED_LINK_TYPES.contains(&kind))
        {
            continue;
        }
        if !is_eligible_interface_name(&link.ifname) {
            continue;
        }
        covered.push(LinuxInterface {
            name: link.ifname.clone(),
            primary_ipv4: by_name
                .get(link.ifname.as_str())
                .copied()
                .and_then(primary_address),
        });
    }
    covered.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(covered)
}
