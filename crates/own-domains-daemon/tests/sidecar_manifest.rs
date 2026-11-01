//! The sidecar manifest is platform-neutral: macOS, Linux, and Windows pin the
//! same CoreDNS/Caddy releases by SHA-256.

use own_domains_daemon::platform::{SidecarManifest, SidecarSpec};

fn manifest_json() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "coredns": {"version": "1.14.7", "sha256": "a".repeat(64)},
        "caddy": {"version": "2.11.7", "sha256": "b".repeat(64)},
    })
}

#[test]
fn shared_manifest_parses_pinned_sidecars() {
    let manifest: SidecarManifest = serde_json::from_value(manifest_json()).unwrap();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(
        manifest.coredns,
        SidecarSpec {
            version: "1.14.7".into(),
            sha256: "a".repeat(64),
        }
    );
    assert_eq!(
        manifest.caddy,
        SidecarSpec {
            version: "2.11.7".into(),
            sha256: "b".repeat(64),
        }
    );
    #[cfg(target_os = "macos")]
    let _same_type: own_domains_daemon::platform::macos::SidecarManifest = manifest;
}
