use own_domains_core::{DomainMapping, Settings};

#[test]
fn deserializes_existing_schema_v1_settings() {
    let input = r#"{
  "schemaVersion": 1,
  "revision": 7,
  "lanAddress": "192.168.1.20",
  "upstreamDns": ["1.1.1.1", "9.9.9.9"],
  "mappings": [
    {
      "id": "018f0000-0000-7000-8000-000000000001",
      "hostname": "shop.own",
      "upstreamPort": 3000,
      "tls": true,
      "enabled": true
    }
  ]
}"#;

    let settings: Settings = serde_json::from_str(input).expect("schema v1 should deserialize");

    assert_eq!(settings.schema_version, 1);
    assert_eq!(settings.revision, 7);
    assert_eq!(settings.lan_address, "192.168.1.20");
    assert_eq!(settings.upstream_dns, ["1.1.1.1", "9.9.9.9"]);
    assert_eq!(
        settings.mappings,
        [DomainMapping {
            id: "018f0000-0000-7000-8000-000000000001".into(),
            hostname: "shop.own".into(),
            upstream_port: 3000,
            tls: true,
            enabled: true,
        }]
    );

    let serialized = serde_json::to_value(&settings).expect("settings should serialize");
    assert_eq!(serialized["schemaVersion"], 1);
    assert_eq!(serialized["revision"], 7);
    assert_eq!(serialized["lanAddress"], "192.168.1.20");
    assert_eq!(serialized["upstreamDns"][0], "1.1.1.1");
    assert_eq!(serialized["mappings"][0]["upstreamPort"], 3000);
    assert_eq!(
        serialized["mappings"][0]["id"],
        "018f0000-0000-7000-8000-000000000001"
    );
}

#[test]
fn default_settings_match_existing_defaults() {
    let settings = Settings::default();

    assert_eq!(settings.schema_version, 1);
    assert_eq!(settings.revision, 0);
    assert_eq!(settings.lan_address, "");
    assert_eq!(settings.upstream_dns, ["1.1.1.1", "9.9.9.9"]);
    assert!(settings.mappings.is_empty());
}
