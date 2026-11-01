//! Linux runtime config constructors (pure rendering, no privileges needed).

use std::net::Ipv4Addr;

use own_domains_core::{CoreResult, DnsConfigOptions, ProxyConfigOptions, Settings};
use own_domains_daemon::platform::linux::{
    linux_dns_options, linux_proxy_options, render_linux_corefile, render_linux_proxy,
};
use serde_json::Value;

fn admin_socket() -> std::path::PathBuf {
    std::env::temp_dir().join("od-linux-test/caddy-admin.sock")
}

fn settings() -> Settings {
    Settings {
        lan_address: "192.168.1.20".into(),
        upstream_dns: vec!["1.1.1.1".into()],
        ..Settings::default()
    }
}

#[test]
fn linux_dns_serves_port_5354_with_ttl_zero() {
    let settings = settings();
    let options = linux_dns_options(&settings).unwrap();

    assert_eq!(options.ttl_seconds, 0);
    assert_eq!(options.port, 5354);
    assert_eq!(options.upstream_port, 53);
    assert_eq!(
        options.listen_addresses,
        vec![Ipv4Addr::LOCALHOST, "192.168.1.20".parse().unwrap()]
    );

    let corefile: CoreResult<String> = render_linux_corefile(&settings);
    let corefile = corefile.unwrap();
    assert!(corefile.contains("0 IN A 192.168.1.20"), "{corefile}");
    assert!(
        corefile.contains("bind 127.0.0.1 192.168.1.20"),
        "{corefile}"
    );
    assert!(corefile.contains("forward . 1.1.1.1:53"), "{corefile}");
    assert!(!corefile.contains("60 IN A"), "{corefile}");
}

#[test]
fn linux_proxy_serves_h1_h2_on_8080_and_8443() {
    let settings = settings();
    let admin_sock = admin_socket();
    let options = linux_proxy_options(&settings, &admin_sock).unwrap();

    assert_eq!(
        options.protocols,
        Some(vec!["h1".to_string(), "h2".to_string()])
    );
    assert_eq!(options.http_port, 8080);
    assert_eq!(options.https_port, 8443);
    assert_eq!(options.setup_port, 4781);
    assert_eq!(
        options.admin_listen,
        format!("unix/{}", admin_socket().display())
    );
    assert_eq!(
        options.listen_addresses,
        vec![Ipv4Addr::LOCALHOST, "192.168.1.20".parse().unwrap()]
    );

    let proxy: CoreResult<Value> = render_linux_proxy(&settings, &admin_sock);
    let proxy = proxy.unwrap();
    let text = serde_json::to_string(&proxy).unwrap();
    assert_eq!(
        text.matches("\"protocols\":[\"h1\",\"h2\"]").count(),
        2,
        "{text}"
    );
    assert_eq!(
        proxy["apps"]["http"]["servers"]["http"]["protocols"],
        serde_json::json!(["h1", "h2"])
    );
    assert_eq!(
        proxy["apps"]["http"]["servers"]["https"]["protocols"],
        serde_json::json!(["h1", "h2"])
    );
    for (server, port) in [("http", 8080), ("https", 8443)] {
        let listen = proxy["apps"]["http"]["servers"][server]["listen"]
            .as_array()
            .unwrap();
        assert!(
            listen.contains(&Value::from(format!("127.0.0.1:{port}"))),
            "{server}: {listen:?}"
        );
        assert!(
            listen.contains(&Value::from(format!("192.168.1.20:{port}"))),
            "{server}: {listen:?}"
        );
    }
    assert_eq!(
        proxy["admin"]["listen"],
        Value::from(format!("unix/{}", admin_socket().display()))
    );
    assert!(!text.contains("0.0.0.0"), "{text}");
    assert!(!text.contains("\"h3\""), "{text}");
    assert!(!text.contains("\"udp\""), "{text}");
}

#[test]
fn shared_defaults_stay_macos_compatible() {
    let settings = settings();
    let dns = DnsConfigOptions::for_settings(&settings).unwrap();
    assert_eq!(dns.ttl_seconds, 60);
    let proxy = ProxyConfigOptions::for_runtime(&settings, &admin_socket()).unwrap();
    assert_eq!(proxy.protocols, None);
}
