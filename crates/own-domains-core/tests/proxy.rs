use std::path::PathBuf;

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
    let admin: PathBuf = std::env::temp_dir().join("od-proxy-admin.sock");
    let options = ProxyConfigOptions::for_runtime(&settings, &admin).unwrap();
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
        serde_json::Value::from(format!("unix/{}", admin.display()))
    );
    assert!(text.contains("127.0.0.1:8080"));
    assert!(text.contains("127.0.0.1:8443"));
}

#[test]
fn protocols_render_only_when_set() {
    let settings = settings();
    let base = ProxyConfigOptions::for_runtime(
        &settings,
        &std::env::temp_dir().join("od-proxy-admin2.sock"),
    )
    .unwrap();
    let plain = serde_json::to_string(&build_proxy_config(&settings, &base).unwrap()).unwrap();
    assert!(!plain.contains("protocols"));
    let options = ProxyConfigOptions {
        protocols: Some(vec!["h1".into(), "h2".into()]),
        ..base
    };
    let text = serde_json::to_string(&build_proxy_config(&settings, &options).unwrap()).unwrap();
    assert!(text.contains("\"protocols\":[\"h1\",\"h2\"]"));
}
