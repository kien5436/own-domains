use std::path::Path;

use own_domains_core::{DomainMapping, ProxyConfigOptions, Settings, build_proxy_config};

fn settings() -> Settings {
    Settings {
        lan_address: "192.168.1.20".into(),
        mappings: vec![
            DomainMapping {
                id: "1".into(),
                hostname: "shop.own".into(),
                upstream_port: 3000,
                tls: false,
                enabled: true,
            },
            DomainMapping {
                id: "2".into(),
                hostname: "api.shop.own".into(),
                upstream_port: 4000,
                tls: true,
                enabled: true,
            },
            DomainMapping {
                id: "3".into(),
                hostname: "off.own".into(),
                upstream_port: 5000,
                tls: false,
                enabled: false,
            },
        ],
        ..Settings::default()
    }
}

#[test]
fn builds_exact_http_https_and_setup_routes_without_management_exposure() {
    let settings = settings();
    let options = ProxyConfigOptions::for_runtime(
        &settings,
        Path::new("/Users/kien/Library/Application Support/OwnDomains/runtime/caddy-admin.sock"),
    )
    .unwrap();
    let config = build_proxy_config(&settings, &options).unwrap();
    let text = serde_json::to_string(&config).unwrap();

    assert!(text.contains("shop.own"));
    assert!(text.contains("api.shop.own"));
    assert!(text.contains("127.0.0.1:3000"));
    assert!(text.contains("127.0.0.1:4000"));
    assert!(text.contains("https://api.shop.own{http.request.uri}"));
    assert!(text.contains("setup.own"));
    assert!(text.contains("192.168.1.20"));
    assert!(text.contains("127.0.0.1:4781"));
    assert!(text.contains("Unknown .own domain"));
    assert!(!text.contains("off.own"));
    assert!(!text.contains("registry.own"));
    assert!(!text.contains("4780"));
    assert_eq!(
        config["admin"]["listen"],
        "unix//Users/kien/Library/Application Support/OwnDomains/runtime/caddy-admin.sock"
    );
    assert!(text.contains("127.0.0.1:8080"));
    assert!(text.contains("127.0.0.1:8443"));
}
