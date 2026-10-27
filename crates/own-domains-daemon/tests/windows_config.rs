#![cfg(target_os = "windows")]

//! Exact Windows host artifact rendering (no privileges needed).

use std::{net::Ipv4Addr, path::Path};

use own_domains_core::{Settings, build_corefile, build_proxy_config};
use own_domains_daemon::platform::windows::{windows_dns_options, windows_proxy_options};

fn settings() -> Settings {
    Settings {
        lan_address: "192.168.1.20".into(),
        upstream_dns: vec!["1.1.1.1".into()],
        ..Settings::default()
    }
}

#[test]
fn windows_dns_binds_loopback_and_lan_on_53_with_zero_ttl() {
    let options = windows_dns_options(&settings()).unwrap();
    assert_eq!(
        options.listen_addresses,
        vec![Ipv4Addr::LOCALHOST, "192.168.1.20".parse().unwrap()]
    );
    assert_eq!(options.port, 53);
    assert_eq!(options.upstream_port, 53);
    assert_eq!(options.ttl_seconds, 0);
    let corefile = build_corefile(&settings(), &options).unwrap();
    assert!(corefile.contains("own:53"), "{corefile}");
    assert!(
        corefile.contains("bind 127.0.0.1 192.168.1.20"),
        "{corefile}"
    );
    assert!(corefile.contains("0 IN A 192.168.1.20"), "{corefile}");
    assert!(!corefile.contains("0.0.0.0"), "{corefile}");
    assert!(!corefile.contains("::"), "{corefile}");
    assert!(corefile.contains("forward . 1.1.1.1:53"), "{corefile}");
}

#[test]
fn windows_proxy_binds_80_and_443_with_h1_h2_only() {
    let admin = std::env::temp_dir().join("od-win-test\\caddy-admin.sock");
    let options = windows_proxy_options(&settings(), &admin).unwrap();
    assert_eq!(
        options.listen_addresses,
        vec![Ipv4Addr::LOCALHOST, "192.168.1.20".parse().unwrap()]
    );
    assert_eq!(options.http_port, 80);
    assert_eq!(options.https_port, 443);
    assert_eq!(options.setup_port, 4781);
    assert_eq!(
        options.protocols,
        Some(vec!["h1".to_string(), "h2".to_string()])
    );
    let config = build_proxy_config(&settings(), &options).unwrap();
    let text = serde_json::to_string(&config).unwrap();
    assert!(text.contains("\"protocols\":[\"h1\",\"h2\"]"), "{text}");
    assert!(text.contains("127.0.0.1:80"), "{text}");
    assert!(text.contains("192.168.1.20:80"), "{text}");
    assert!(text.contains("127.0.0.1:443"), "{text}");
    assert!(text.contains("192.168.1.20:443"), "{text}");
    assert!(!text.contains("0.0.0.0"), "{text}");
    assert_eq!(
        config["admin"]["listen"],
        serde_json::Value::from(format!("unix/{}", admin.display()))
    );
    assert!(windows_proxy_options(&settings(), Path::new("relative.sock")).is_err());
}
