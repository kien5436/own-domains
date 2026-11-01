//! Covered interface validation for the Linux nftables redirect.
//!
//! Names are validated, sorted, unique, and capped. The denylist runs before
//! the allowlist; Docker's `br-` plus 12 hex digits is checked before the
//! `br` allowlist prefix so bridge interfaces stay accepted.

use super::error::LinuxArtifactError;

pub const MAX_INTERFACES: usize = 32;
pub const MAX_INTERFACE_NAME_BYTES: usize = 15;

const ALLOWLIST_PREFIXES: &[&str] = &["en", "eth", "wl", "wlan", "br", "bond", "vlan"];

const DENYLIST_EXACT: &[&str] = &["lo"];

const DENYLIST_PREFIXES: &[&str] = &[
    "docker",
    "veth",
    "virbr",
    "tun",
    "tap",
    "wg",
    "tailscale",
    "zt",
    "podman",
    "cni",
    "flannel.",
    "cali",
    "lxc",
    "lxd",
    "incus",
    "vmnet",
    "vboxnet",
    "dummy",
    "sit",
    "ip6tnl",
    "gre",
    "gretap",
    "erspan",
    "ipip",
    "ifb",
];

fn is_docker_bridge(name: &str) -> bool {
    match name.strip_prefix("br-") {
        Some(rest) => rest.len() == 12 && rest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        None => false,
    }
}

pub fn is_eligible_interface_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_INTERFACE_NAME_BYTES {
        return false;
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return false;
    }
    if DENYLIST_EXACT.contains(&name) {
        return false;
    }
    if is_docker_bridge(name) {
        return false;
    }
    if DENYLIST_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return false;
    }
    ALLOWLIST_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

pub fn validate_covered_interfaces<N: AsRef<str>>(
    names: &[N],
) -> Result<Vec<String>, LinuxArtifactError> {
    if names.is_empty() {
        return Err(LinuxArtifactError::EmptyInterfaces);
    }
    if names.len() > MAX_INTERFACES {
        return Err(LinuxArtifactError::TooManyInterfaces(names.len()));
    }
    let mut covered = Vec::with_capacity(names.len());
    for name in names {
        let name = name.as_ref();
        if !is_eligible_interface_name(name) {
            return Err(LinuxArtifactError::InvalidInterface(name.to_string()));
        }
        if covered.iter().any(|known: &String| known.as_str() == name) {
            return Err(LinuxArtifactError::DuplicateInterface(name.to_string()));
        }
        covered.push(name.to_string());
    }
    covered.sort();
    Ok(covered)
}
