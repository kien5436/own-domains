use own_domains_core::{
    CoreError, DomainMapping, MappingInput, Settings, normalize_hostname, validate_hostname,
    validate_mapping, validate_network, validate_settings,
};

fn mapping(hostname: &str, upstream_port: u32) -> MappingInput {
    MappingInput {
        id: None,
        hostname: hostname.into(),
        upstream_port,
        tls: false,
        enabled: true,
    }
}

fn persisted_mapping(id: &str, hostname: &str) -> DomainMapping {
    DomainMapping {
        id: id.into(),
        hostname: hostname.into(),
        upstream_port: 3000,
        tls: false,
        enabled: true,
    }
}

#[test]
fn normalizes_and_accepts_nested_hostnames() {
    assert_eq!(normalize_hostname("Shop.OWN."), "shop.own");
    assert_eq!(normalize_hostname("Shop.OWN"), "shop.own");
    assert_eq!(
        validate_hostname("admin.api.shop.own").unwrap(),
        "admin.api.shop.own"
    );
}

#[test]
fn rejects_invalid_and_reserved_hostnames() {
    let long_label = format!("{}.own", "a".repeat(64));
    let long_hostname = format!("{}.own", "a.".repeat(125));
    let invalid = [
        "shop..own",
        "café.own",
        "*.shop.own",
        "127.0.0.1",
        "setup.own",
        "api.setup.own",
        "registry.own",
        long_label.as_str(),
        long_hostname.as_str(),
    ];

    for hostname in invalid {
        assert!(
            validate_mapping(mapping(hostname, 3000)).is_err(),
            "{hostname} should fail"
        );
    }
}

#[test]
fn rejects_reserved_and_out_of_range_ports() {
    for port in [0, 53, 80, 443, 4780, 65_536] {
        assert!(
            validate_mapping(mapping("shop.own", port)).is_err(),
            "{port} should fail"
        );
    }
}

#[test]
fn validates_network_addresses_and_forwarding_loops() {
    assert!(validate_network("192.168.1.20", &["1.1.1.1".into(), "9.9.9.9".into()]).is_ok());

    let invalid = [
        ("", vec!["1.1.1.1".into()]),
        ("::1", vec!["1.1.1.1".into()]),
        ("not-an-ip", vec!["1.1.1.1".into()]),
        ("192.168.1.20", vec![]),
        ("192.168.1.20", vec!["2606:4700:4700::1111".into()]),
        ("192.168.1.20", vec!["127.0.0.1".into()]),
        ("192.168.1.20", vec!["192.168.1.20".into()]),
    ];

    for (lan, upstreams) in invalid {
        assert!(
            validate_network(lan, &upstreams).is_err(),
            "{lan} {upstreams:?} should fail"
        );
    }
}

#[test]
fn rejects_unsupported_schema_and_duplicate_identity() {
    let unsupported = Settings {
        schema_version: 2,
        ..Settings::default()
    };
    assert!(validate_settings(unsupported).is_err());

    let duplicate_ids = Settings {
        mappings: vec![
            persisted_mapping("same", "one.own"),
            persisted_mapping("same", "two.own"),
        ],
        ..Settings::default()
    };
    assert!(validate_settings(duplicate_ids).is_err());

    let duplicate_hosts = Settings {
        mappings: vec![
            persisted_mapping("one", "Shop.OWN."),
            persisted_mapping("two", "shop.own"),
        ],
        ..Settings::default()
    };
    assert!(matches!(
        validate_settings(duplicate_hosts),
        Err(CoreError::DuplicateHostname { hostname }) if hostname == "shop.own"
    ));
}

#[test]
fn loads_legacy_registry_mapping_but_does_not_allow_new_registration() {
    let mut settings = Settings::default();
    settings
        .mappings
        .push(persisted_mapping("legacy", "registry.own"));

    assert!(validate_settings(settings).is_ok());
    assert!(validate_mapping(mapping("registry.own", 3000)).is_err());
}
