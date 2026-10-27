//! The readiness probes and Caddy admin client are transport- and
//! platform-neutral: the same types serve macOS, Linux, and Windows.

use own_domains_daemon::probe::{
    CaddyAdminClient, DnsReadinessProbe, HttpReadinessProbe, ReadinessProbe,
};
use std::{
    net::Ipv4Addr,
    path::Path,
    time::{Duration, Instant},
};

#[test]
fn caddy_admin_client_requires_an_absolute_socket() {
    assert!(CaddyAdminClient::new(Path::new("relative.sock")).is_err());
    assert!(CaddyAdminClient::new(std::env::temp_dir().join("admin.sock")).is_ok());
}

#[test]
fn readiness_probes_fail_fast_against_a_closed_loopback_port() {
    let port = reserved_loopback_port();
    // Unconnected UDP learns of a closed port only via read timeout, so keep
    // this short: the assertion is "errors, not hangs".
    let timeout = Duration::from_secs(1);
    let start = Instant::now();
    let dns = DnsReadinessProbe::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST, timeout);
    assert!(dns.check().is_err());
    let http = HttpReadinessProbe::new(
        Ipv4Addr::LOCALHOST,
        port,
        "setup.own".to_string(),
        "setup.own".to_string(),
        200,
        timeout,
    );
    assert!(http.check().is_err());
    // Both probes must fail rather than hang; allow generous scheduling slack.
    assert!(start.elapsed() < timeout * 5);
}

fn reserved_loopback_port() -> u16 {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}
