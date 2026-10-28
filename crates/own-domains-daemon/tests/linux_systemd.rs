//! systemd unit rendering against byte-exact fixtures.
//!
//! Static text from the design: no user input, no addresses, no substitution.

use own_domains_daemon::platform::linux::systemd::{
    NFTABLES_UNIT_NAME, RESOLVER_UNIT_NAME, render_nftables_unit, render_resolver_unit,
};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux")
            .join(name),
    )
    .unwrap()
}

const HARDENING: &[&str] = &[
    "Type=oneshot",
    "RemainAfterExit=yes",
    "NoNewPrivileges=yes",
    "PrivateTmp=yes",
    "ProtectHome=yes",
    "ProtectSystem=full",
    "ProtectKernelTunables=yes",
    "ProtectControlGroups=yes",
    "CapabilityBoundingSet=CAP_NET_ADMIN",
];

fn assert_hardened(unit: &str) {
    for directive in HARDENING {
        assert!(unit.contains(directive), "missing {directive}");
    }
    assert!(!unit.contains("192.168."), "no LAN address");
    assert!(!unit.contains('$'), "no shell substitution");
    assert!(!unit.contains('{'), "no template substitution");
    assert!(unit.ends_with('\n') && !unit.ends_with("\n\n"));
}

#[test]
fn nftables_unit_is_byte_exact() {
    assert_eq!(NFTABLES_UNIT_NAME, "own-domains-nftables.service");
    let rendered = render_nftables_unit();
    assert_eq!(rendered, fixture("own-domains-nftables.service.expected"));
    assert_hardened(&rendered);
    for line in [
        "After=network-pre.target nftables.service",
        "Before=network.target",
        "ExecStart=/usr/sbin/nft -f /etc/own-domains/nftables.conf",
        "ExecStop=-/usr/sbin/nft delete table ip own_domains",
    ] {
        assert!(rendered.contains(line), "missing {line}");
    }
}

#[test]
fn resolver_unit_is_byte_exact() {
    assert_eq!(RESOLVER_UNIT_NAME, "own-domains-resolver.service");
    let rendered = render_resolver_unit();
    assert_eq!(rendered, fixture("own-domains-resolver.service.expected"));
    assert_hardened(&rendered);
    for line in [
        "After=systemd-resolved.service",
        "PartOf=systemd-resolved.service",
        "ExecStart=-/usr/sbin/ip link add own-domains0 type dummy",
        "ExecStart=/usr/bin/resolvectl dns own-domains0 127.0.0.1:5354",
        "ExecStart=/usr/bin/resolvectl domain own-domains0 \"~own\"",
        "ExecStart=/usr/bin/resolvectl dnssec own-domains0 no",
        "ExecStart=/usr/bin/resolvectl dnsovertls own-domains0 no",
        "ExecStart=/usr/bin/resolvectl default-route own-domains0 no",
        "ExecStop=-/usr/bin/resolvectl revert own-domains0",
        "ExecStop=-/usr/sbin/ip link delete own-domains0",
    ] {
        assert!(rendered.contains(line), "missing {line}");
    }
}
