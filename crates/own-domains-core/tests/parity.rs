use std::fs;

use own_domains_core::{
    CoreError, DnsConfigOptions, ProxyConfigOptions, Settings, SettingsStore, build_corefile,
    build_proxy_config, validate_settings,
};
use uuid::Uuid;

#[test]
fn shared_schema_v1_fixture_preserves_semantics_and_generates_configs() {
    let source = include_str!("../../../tests/fixtures/core-parity/settings-valid.json");
    let settings = validate_settings(serde_json::from_str::<Settings>(source).unwrap()).unwrap();
    assert_eq!(settings.revision, 8);
    assert_eq!(settings.mappings[1].hostname, "api.shop.own");
    let proxy = serde_json::to_string(
        &build_proxy_config(
            &settings,
            &ProxyConfigOptions::for_settings(&settings).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(proxy.contains("127.0.0.1:3000"));
    assert!(proxy.contains("https://api.shop.own"));
    assert!(!proxy.contains("disabled.own"));
    assert!(!proxy.contains("registry.own"));
    let corefile = build_corefile(
        &settings,
        &DnsConfigOptions::for_settings(&settings).unwrap(),
    )
    .unwrap();
    assert!(corefile.contains("192.168.1.20"));
    assert!(corefile.contains("template IN AAAA"));

    let directory = std::env::temp_dir().join(format!("own-domains-parity-{}", Uuid::new_v4()));
    let store = SettingsStore::new(&directory);
    store.save(&settings, Some(0)).unwrap();
    assert_eq!(store.load().unwrap(), settings);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn shared_duplicate_fixture_has_typed_error() {
    let source =
        include_str!("../../../tests/fixtures/core-parity/settings-invalid-duplicate.json");
    let settings = serde_json::from_str::<Settings>(source).unwrap();
    assert!(
        matches!(validate_settings(settings), Err(CoreError::DuplicateHostname { hostname }) if hostname == "shop.own")
    );
}
