//! nftables ruleset rendering for the Linux redirect table.
//!
//! Pure text: interfaces validated, sorted, unique, and capped by
//! [`validate_covered_interfaces`]. No LAN address appears anywhere; the
//! redirect targets are the fixed high ports.

use super::{error::LinuxArtifactError, interfaces::validate_covered_interfaces};

pub const NFTABLES_TABLE: &str = "ip own_domains";

const DNS_REDIRECT_PORT: u16 = 5354;
const HTTP_REDIRECT_PORT: u16 = 8080;
const HTTPS_REDIRECT_PORT: u16 = 8443;

pub fn render_nftables<N: AsRef<str>>(
    interfaces: &[N],
    user_id: u32,
) -> Result<String, LinuxArtifactError> {
    if user_id == 0 {
        return Err(LinuxArtifactError::InvalidUserId);
    }
    let covered = validate_covered_interfaces(interfaces)?;

    let mut ruleset = String::new();
    ruleset.push_str(&format!(
        "destroy table {NFTABLES_TABLE}\n\ntable {NFTABLES_TABLE} {{\n"
    ));
    ruleset.push_str(
        "  chain prerouting {\n    type nat hook prerouting priority -110; policy accept;\n",
    );
    for name in &covered {
        ruleset.push('\n');
        for (protocol, dport, to) in [
            ("udp", 53, DNS_REDIRECT_PORT),
            ("tcp", 53, DNS_REDIRECT_PORT),
            ("tcp", 80, HTTP_REDIRECT_PORT),
            ("tcp", 443, HTTPS_REDIRECT_PORT),
        ] {
            ruleset.push_str(&format!(
                "    iifname \"{name}\" fib daddr . iif type local ip daddr != 169.254.0.0/16 {protocol} dport {dport} redirect to :{to}\n"
            ));
        }
    }
    ruleset.push_str(
        "  }\n\n  chain output {\n    type nat hook output priority -100; policy accept;\n\n",
    );
    for (dport, to) in [(80, HTTP_REDIRECT_PORT), (443, HTTPS_REDIRECT_PORT)] {
        ruleset.push_str(&format!(
            "    meta skuid {user_id} fib daddr type local ip daddr != 127.0.0.0/8 ip daddr != 169.254.0.0/16 tcp dport {dport} redirect to :{to}\n"
        ));
    }
    ruleset.push_str("  }\n}\n");
    Ok(ruleset)
}
