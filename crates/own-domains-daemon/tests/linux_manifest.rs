//! Linux install manifest rendering and validation.
//!
//! The expected manifest is checked in; artifact digests below were produced
//! with `shasum -a 256`, not copied from renderer output.

use own_domains_daemon::platform::{
    SidecarManifest, SidecarSpec,
    linux::manifest::{
        LinuxArtifactContent, LinuxArtifactKind, LinuxInstallManifest, build_linux_manifest,
        parse_linux_manifest, render_linux_manifest, validate_linux_manifest,
    },
};

const CA_FINGERPRINT: &str = "15a63910f999a5e2e8493ccf7e57b0c7c85fccc70ee8eb62d8f309a4e013be6f";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux")
            .join(name),
    )
    .unwrap()
}

fn certificate_pem() -> &'static [u8] {
    include_bytes!("../../../tests/fixtures/sidecars/certificate.pem")
}

fn sidecars() -> SidecarManifest {
    SidecarManifest {
        schema_version: 1,
        coredns: SidecarSpec {
            version: "1.14.7".into(),
            sha256: "a".repeat(64),
        },
        caddy: SidecarSpec {
            version: "2.11.7".into(),
            sha256: "b".repeat(64),
        },
    }
}

fn artifacts() -> (String, String, String) {
    (
        fixture("nftables.conf.expected"),
        fixture("own-domains-nftables.service.expected"),
        fixture("own-domains-resolver.service.expected"),
    )
}

fn valid_manifest() -> LinuxInstallManifest {
    let (nftables, nftables_unit, resolver_unit) = artifacts();
    build_linux_manifest(
        "alice",
        1000,
        &["wlp2s0", "enp3s0"],
        sidecars(),
        CA_FINGERPRINT,
        &[
            LinuxArtifactContent {
                path: "/etc/own-domains/nftables.conf",
                kind: LinuxArtifactKind::Nftables,
                content: nftables.as_bytes(),
            },
            LinuxArtifactContent {
                path: "/etc/systemd/system/own-domains-nftables.service",
                kind: LinuxArtifactKind::SystemdUnit,
                content: nftables_unit.as_bytes(),
            },
            LinuxArtifactContent {
                path: "/etc/systemd/system/own-domains-resolver.service",
                kind: LinuxArtifactKind::SystemdUnit,
                content: resolver_unit.as_bytes(),
            },
            LinuxArtifactContent {
                path: "/usr/local/share/ca-certificates/own-domains.crt",
                kind: LinuxArtifactKind::TrustCertificate,
                content: certificate_pem(),
            },
        ],
    )
}

#[test]
fn manifest_is_byte_exact_and_round_trips() {
    let expected = fixture("install-manifest.json.expected");
    let manifest = valid_manifest();
    let rendered = render_linux_manifest(&manifest).unwrap();
    assert_eq!(rendered, expected);
    let parsed = parse_linux_manifest(rendered.as_bytes()).unwrap();
    assert_eq!(parsed, manifest);
    validate_linux_manifest(&parsed).unwrap();
}

fn assert_invalid(mutate: impl FnOnce(&mut LinuxInstallManifest)) {
    let mut manifest = valid_manifest();
    mutate(&mut manifest);
    assert!(validate_linux_manifest(&manifest).is_err(), "{manifest:?}");
    let bytes = serde_json::to_vec(&manifest).unwrap();
    assert!(parse_linux_manifest(&bytes).is_err());
}

#[test]
fn validation_rejects_bad_artifacts() {
    assert_invalid(|manifest| {
        manifest.artifacts.push(manifest.artifacts[0].clone());
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].path = "/etc/passwd".into();
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].kind = LinuxArtifactKind::TrustCertificate;
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].mode = 0o600;
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].owner_id = 1000;
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].group_id = 1000;
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].sha256 = "ABCDEF".repeat(10) + "AB";
    });
    assert_invalid(|manifest| {
        manifest.artifacts[0].sha256 = "a".repeat(63);
    });
}

#[test]
fn validation_rejects_bad_interfaces() {
    assert_invalid(|manifest| manifest.interfaces.clear());
    assert_invalid(|manifest| manifest.interfaces = vec!["eth0".into(), "eth0".into()]);
    assert_invalid(|manifest| manifest.interfaces = vec!["wlp2s0".into(), "enp3s0".into()]);
    assert_invalid(|manifest| {
        manifest.interfaces = (0..33).map(|index| format!("eth{index}")).collect();
    });
    assert_invalid(|manifest| manifest.interfaces = vec!["docker0".into()]);
}

#[test]
fn validation_rejects_bad_metadata() {
    assert_invalid(|manifest| manifest.backup_directory = "/tmp/backups".into());
    assert_invalid(|manifest| manifest.nftables_table = "ip filter".into());
    assert_invalid(|manifest| manifest.resolver_link = "own-domains1".into());
    assert_invalid(|manifest| {
        manifest.systemd_units = vec![
            "other.service".into(),
            "own-domains-resolver.service".into(),
        ]
    });
    assert_invalid(|manifest| manifest.platform = "darwin".into());
    assert_invalid(|manifest| manifest.schema_version = 2);
    assert_invalid(|manifest| manifest.host_integration_version = 2);
    assert_invalid(|manifest| manifest.user_id = 0);
    assert_invalid(|manifest| manifest.user_name.clear());
    assert_invalid(|manifest| manifest.user_name = "bad name!".into());
    assert_invalid(|manifest| manifest.sidecars.schema_version = 2);
    assert_invalid(|manifest| {
        manifest.sidecars.coredns.sha256 = "A".repeat(64);
    });
    assert_invalid(|manifest| {
        manifest.sidecars.caddy.sha256 = "b".repeat(63);
    });
    assert_invalid(|manifest| {
        manifest.ca_fingerprint_sha256 = "F".repeat(64);
    });
    assert_invalid(|manifest| {
        manifest.ca_fingerprint_sha256 = "15a63910".into();
    });
}
