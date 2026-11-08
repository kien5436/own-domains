//! Firewall coexistence parsing shared by setup preflight and the
//! host inspector.
//!
//! Pure functions over fixed command outputs; no commands run here.
//! A NAT redirect does not bypass the host firewall: after translation
//! the input filter sees the high ports, so an active default-deny
//! firewall must explicitly allow them. Nothing here mutates firewall
//! state; the remedy is always a command for the operator to run.

/// High ports the runtime binds (post-redirect view of the firewall).
pub const REQUIRED_PORT_RULES: [(&str, &str); 4] = [
    ("5354", "tcp"),
    ("5354", "udp"),
    ("8080", "tcp"),
    ("8443", "tcp"),
];

/// Parsed firewall posture: whether a default-deny policy is enforced
/// and which `port/proto` rules are already allowed.
pub struct FirewallPolicy {
    pub enforced: bool,
    pub allowed: Vec<String>,
}

/// Parse `ufw status`. Inactive output (or anything without the active
/// header) yields an unenforced policy rather than an error.
pub fn parse_ufw_status(output: &str) -> FirewallPolicy {
    let active = output.lines().any(|line| line.trim() == "Status: active");
    if !active {
        return FirewallPolicy {
            enforced: false,
            allowed: Vec::new(),
        };
    }
    let default_deny = output
        .lines()
        .any(|line| line.contains("Default:") && line.contains("deny (incoming)"));
    let allowed = output
        .lines()
        .filter(|line| line.contains("ALLOW"))
        .flat_map(|line| {
            line.split_whitespace()
                .filter(|token| {
                    token.contains('/')
                        && token
                            .chars()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == '/')
                })
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    FirewallPolicy {
        enforced: default_deny,
        allowed,
    }
}

/// Parse `firewall-cmd --list-ports` (whitespace-separated `port/proto`).
pub fn parse_firewalld_ports(output: &str) -> Vec<String> {
    output
        .split_whitespace()
        .filter(|entry| entry.contains('/'))
        .map(str::to_string)
        .collect()
}

/// Required rules absent from `allowed` (exact `port/proto` match).
pub fn missing_port_rules(allowed: &[String]) -> Vec<(String, String)> {
    REQUIRED_PORT_RULES
        .iter()
        .filter(|(port, protocol)| {
            let rule = format!("{port}/{protocol}");
            !allowed.iter().any(|entry| entry == &rule)
        })
        .map(|(port, protocol)| (port.to_string(), protocol.to_string()))
        .collect()
}

/// Exact operator commands for UFW gaps.
pub fn ufw_remedy(missing: &[(String, String)]) -> String {
    missing
        .iter()
        .map(|(port, protocol)| format!("sudo ufw allow {port}/{protocol}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exact operator commands for a firewalld gap in `zone`.
pub fn firewalld_remedy(zone: &str, missing: &[(String, String)]) -> String {
    let mut lines: Vec<String> = missing
        .iter()
        .map(|(port, protocol)| {
            format!("sudo firewall-cmd --permanent --zone={zone} --add-port={port}/{protocol}")
        })
        .collect();
    lines.push("sudo firewall-cmd --reload".into());
    lines.join("\n")
}
