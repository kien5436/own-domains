//! Covered interface validation, tested on every platform.
//!
//! Pure allowlist/denylist logic: no root, no `ip` calls.

use own_domains_daemon::platform::linux::{
    LinuxArtifactError,
    interfaces::{is_eligible_interface_name, validate_covered_interfaces},
};

const ACCEPTED: &[&str] = &[
    "enp3s0",
    "eth0",
    "wlp2s0",
    "wlan0",
    "br0",
    "bond0",
    "vlan100",
    "eno1",
    "ens3",
    "wlx001122334455",
];

const REJECTED: &[&str] = &[
    "lo",
    "docker0",
    "br-0123456789ab",
    "veth123",
    "virbr0",
    "tun0",
    "tap0",
    "wg0",
    "tailscale0",
    "ztabc",
    "podman0",
    "cni0",
    "flannel.1",
    "cali123",
    "lxcbr0",
    "lxdbr0",
    "incusbr0",
    "vmnet1",
    "vboxnet0",
    "dummy0",
    "sit0",
    "ip6tnl0",
    "gre0",
    "gretap0",
    "erspan0",
    "ipip0",
    "ifb0",
    "",
    "enp3s01234567890",
    "eth/0",
    "eth 0",
    "eth;0",
    "eth\"0",
];

#[test]
fn eligible_names_cover_physical_bridge_and_bond_kinds() {
    for name in ACCEPTED {
        assert!(is_eligible_interface_name(name), "{name}");
    }
}

#[test]
fn virtual_tunnel_and_overlong_names_are_rejected() {
    for name in REJECTED {
        assert!(!is_eligible_interface_name(name), "{name}");
    }
}

#[test]
fn covered_interfaces_are_sorted_unique_and_capped() {
    assert_eq!(
        validate_covered_interfaces(&["wlp2s0", "enp3s0"]).unwrap(),
        vec!["enp3s0".to_string(), "wlp2s0".to_string()]
    );
    assert!(matches!(
        validate_covered_interfaces::<&str>(&[]),
        Err(LinuxArtifactError::EmptyInterfaces)
    ));
    assert!(matches!(
        validate_covered_interfaces(&["eth0", "eth0"]),
        Err(LinuxArtifactError::DuplicateInterface(_))
    ));
    let many: Vec<String> = (0..33).map(|index| format!("eth{index}")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(matches!(
        validate_covered_interfaces(&many_refs),
        Err(LinuxArtifactError::TooManyInterfaces(33))
    ));
    assert!(matches!(
        validate_covered_interfaces(&["eth0", "docker0"]),
        Err(LinuxArtifactError::InvalidInterface(_))
    ));
}
