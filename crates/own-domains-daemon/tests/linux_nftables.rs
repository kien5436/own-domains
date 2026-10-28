//! nftables ruleset rendering against a byte-exact fixture.
//!
//! Pure rendering: no root, no `nft`.

use own_domains_daemon::platform::linux::nftables::render_nftables;

fn fixture() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux/nftables.conf.expected"),
    )
    .unwrap()
}

#[test]
fn ruleset_is_byte_exact_and_sorted() {
    let expected = fixture();
    assert!(expected.ends_with("}\n") && !expected.ends_with("}\n\n"));
    assert!(
        !expected.contains("192.168"),
        "fixture must not embed a LAN address"
    );
    assert!(!expected.contains("0.0.0.0"), "no wildcard listeners");
    assert!(
        !expected
            .lines()
            .any(|line| line.contains("output") && line.contains("dport 53")),
        "no output-chain port 53 rule"
    );
    assert!(
        !expected.contains("chain filter"),
        "no filter chain: nat redirect only"
    );
    let rendered = render_nftables(&["wlp2s0", "enp3s0"], 1000).unwrap();
    assert_eq!(rendered, expected);
}

#[test]
fn rendering_rejects_bad_inputs() {
    assert!(render_nftables::<&str>(&[], 1000).is_err());
    assert!(render_nftables(&["eth0", "eth0"], 1000).is_err());
    assert!(render_nftables(&["eth0", "docker0"], 1000).is_err());
    let many: Vec<String> = (0..33).map(|index| format!("eth{index}")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(render_nftables(&many_refs, 1000).is_err());
    assert!(render_nftables(&["eth0"], 0).is_err());
    assert!(
        render_nftables(&["eth0"], 1000)
            .unwrap()
            .contains("meta skuid 1000")
    );
}
