//! `ip -j` discovery against checked-in fixtures (no root, no `ip`).
//!
//! The fixtures cover physical, bridge, bond, and VLAN interfaces with and
//! without addresses, plus slaves and virtual interfaces that must be
//! excluded.

use own_domains_daemon::platform::linux::ip::{LinuxInterface, discover_covered_interfaces};
use std::net::Ipv4Addr;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux")
            .join(name),
    )
    .unwrap()
}

fn address(value: &str) -> Ipv4Addr {
    value.parse().unwrap()
}

#[test]
fn discovery_returns_covered_interfaces_with_primaries() {
    let covered =
        discover_covered_interfaces(&fixture("ip-link.json"), &fixture("ip-addr.json")).unwrap();
    assert_eq!(
        covered
            .iter()
            .map(|interface| interface.name.as_str())
            .collect::<Vec<_>>(),
        vec!["bond0", "br0", "enp3s0", "vlan100", "wlp2s0"]
    );
    assert_eq!(
        covered
            .iter()
            .map(|interface| interface.primary_ipv4)
            .collect::<Vec<_>>(),
        vec![
            None,
            Some(address("10.0.0.5")),
            Some(address("192.168.1.20")),
            Some(address("10.0.0.100")),
            None,
        ]
    );
    // The secondary alias is never selected, even though it sorts first.
    assert!(
        !covered
            .iter()
            .any(|interface| interface.primary_ipv4 == Some(address("192.168.1.21")))
    );
}

#[test]
fn malformed_json_is_an_error() {
    assert!(discover_covered_interfaces("not json", &fixture("ip-addr.json")).is_err());
    assert!(discover_covered_interfaces(&fixture("ip-link.json"), "[{\"ifname\": 1}]").is_err());
}

#[test]
fn interface_shape_is_name_and_optional_primary() {
    let interface = LinuxInterface {
        name: "eth0".to_string(),
        primary_ipv4: None,
    };
    assert_eq!(interface.name, "eth0");
    assert_eq!(interface.primary_ipv4, None);
}
