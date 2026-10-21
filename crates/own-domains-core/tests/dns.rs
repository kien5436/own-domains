use std::net::Ipv4Addr;

use own_domains_core::{DnsConfigOptions, Settings, build_corefile};

fn settings() -> Settings {
    Settings {
        lan_address: "192.168.1.20".into(),
        ..Settings::default()
    }
}

#[test]
fn builds_exact_authoritative_corefile() {
    let settings = settings();
    let options = DnsConfigOptions {
        listen_addresses: vec![Ipv4Addr::LOCALHOST, "192.168.1.20".parse().unwrap()],
        port: 1053,
        upstream_port: 5353,
    };
    assert_eq!(
        build_corefile(&settings, &options).unwrap(),
        include_str!("fixtures/Corefile.expected")
    );
}

#[test]
fn rejects_invalid_loops_and_listener_exposure() {
    let mut looping = settings();
    looping.upstream_dns = vec!["192.168.1.20".into()];
    assert!(DnsConfigOptions::for_settings(&looping).is_err());

    let settings = settings();
    let exposed = DnsConfigOptions {
        listen_addresses: vec!["10.0.0.2".parse().unwrap()],
        port: 53,
        upstream_port: 53,
    };
    assert!(build_corefile(&settings, &exposed).is_err());
    let output = build_corefile(
        &settings,
        &DnsConfigOptions::for_settings(&settings).unwrap(),
    )
    .unwrap();
    assert!(!output.contains("registry.own"));
    assert!(output.ends_with('\n'));
}

#[test]
fn runtime_defaults_use_the_pf_redirect_target_port() {
    let options = DnsConfigOptions::for_settings(&settings()).unwrap();
    assert_eq!(options.port, 5354);
    assert_eq!(options.upstream_port, 53);
}
