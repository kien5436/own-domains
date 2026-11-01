//! Static systemd units for the Linux host integration.
//!
//! Copied from the design verbatim: fixed commands, fixed arguments, no user
//! input, no substitution. The byte-exact fixtures carry the second copy.

pub const NFTABLES_UNIT_NAME: &str = "own-domains-nftables.service";
pub const RESOLVER_UNIT_NAME: &str = "own-domains-resolver.service";

pub fn render_nftables_unit() -> String {
    concat!(
        "[Unit]\n",
        "Description=Own Domains nftables redirect\n",
        "After=network-pre.target nftables.service\n",
        "Wants=network-pre.target\n",
        "Before=network.target\n",
        "\n",
        "[Service]\n",
        "Type=oneshot\n",
        "RemainAfterExit=yes\n",
        "NoNewPrivileges=yes\n",
        "PrivateTmp=yes\n",
        "ProtectHome=yes\n",
        "ProtectSystem=full\n",
        "ProtectKernelTunables=yes\n",
        "ProtectControlGroups=yes\n",
        "CapabilityBoundingSet=CAP_NET_ADMIN\n",
        "ExecStart=/usr/sbin/nft -f /etc/own-domains/nftables.conf\n",
        "ExecStop=-/usr/sbin/nft delete table ip own_domains\n",
        "\n",
        "[Install]\n",
        "WantedBy=multi-user.target\n",
    )
    .to_string()
}

pub fn render_resolver_unit() -> String {
    concat!(
        "[Unit]\n",
        "Description=Own Domains split DNS for .own\n",
        "After=systemd-resolved.service\n",
        "Wants=systemd-resolved.service\n",
        "PartOf=systemd-resolved.service\n",
        "\n",
        "[Service]\n",
        "Type=oneshot\n",
        "RemainAfterExit=yes\n",
        "NoNewPrivileges=yes\n",
        "PrivateTmp=yes\n",
        "ProtectHome=yes\n",
        "ProtectSystem=full\n",
        "ProtectKernelTunables=yes\n",
        "ProtectControlGroups=yes\n",
        "CapabilityBoundingSet=CAP_NET_ADMIN\n",
        "ExecStart=-/usr/sbin/ip link add own-domains0 type dummy\n",
        "ExecStart=/usr/sbin/ip link set own-domains0 up\n",
        "ExecStart=/usr/bin/resolvectl dns own-domains0 127.0.0.1:5354\n",
        "ExecStart=/usr/bin/resolvectl domain own-domains0 \"~own\"\n",
        "ExecStart=/usr/bin/resolvectl dnssec own-domains0 no\n",
        "ExecStart=/usr/bin/resolvectl dnsovertls own-domains0 no\n",
        "ExecStart=/usr/bin/resolvectl default-route own-domains0 no\n",
        "ExecStop=-/usr/bin/resolvectl revert own-domains0\n",
        "ExecStop=-/usr/sbin/ip link delete own-domains0\n",
        "\n",
        "[Install]\n",
        "WantedBy=multi-user.target\n",
    )
    .to_string()
}
