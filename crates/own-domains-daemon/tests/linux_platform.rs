//! Linux backend skeleton: manifest-gated diagnose over a state directory.
//!
//! No processes are started; the full runtime lands in Milestone 2.

use own_domains_core::{DiagnosticStatus, Settings};
use own_domains_daemon::{
    RuntimeBackend, RuntimeError,
    platform::{
        SidecarManifest, SidecarSpec,
        linux::{
            manifest::{LinuxArtifactContent, LinuxArtifactKind, build_linux_manifest},
            runtime::LinuxRuntimeBackend,
        },
    },
};

fn state_dir(tag: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("od-linux-platform-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn write_manifest(directory: &std::path::Path) -> String {
    let nftables = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux/nftables.conf.expected"),
    )
    .unwrap();
    let nftables_unit = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux/own-domains-nftables.service.expected"),
    )
    .unwrap();
    let resolver_unit = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux/own-domains-resolver.service.expected"),
    )
    .unwrap();
    let certificate = include_bytes!("../../../tests/fixtures/sidecars/certificate.pem");
    let manifest = build_linux_manifest(
        "alice",
        1000,
        &["wlp2s0", "enp3s0"],
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
        },
        "15a63910f999a5e2e8493ccf7e57b0c7c85fccc70ee8eb62d8f309a4e013be6f",
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
                content: certificate,
            },
        ],
    );
    let rendered =
        own_domains_daemon::platform::linux::manifest::render_linux_manifest(&manifest).unwrap();
    std::fs::write(directory.join("install-manifest.json"), &rendered).unwrap();
    rendered
}

fn assert_gate(diagnostics: &[own_domains_core::Diagnostic], code: &str) {
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, code);
    assert_eq!(diagnostics[0].status, DiagnosticStatus::Error);
    assert!(!diagnostics[0].message.is_empty());
    assert!(
        diagnostics[0]
            .remedy
            .as_ref()
            .is_some_and(|remedy| !remedy.is_empty())
    );
}

#[test]
fn manifest_gate_reports_setup_required_invalid_or_silence() {
    let missing = state_dir("missing");
    let backend = LinuxRuntimeBackend::new(&missing).unwrap();
    assert_gate(
        &backend.diagnose(&Settings::default()).unwrap(),
        "setupRequired",
    );

    let corrupt = state_dir("corrupt");
    std::fs::write(corrupt.join("install-manifest.json"), "not json").unwrap();
    let backend = LinuxRuntimeBackend::new(&corrupt).unwrap();
    assert_gate(
        &backend.diagnose(&Settings::default()).unwrap(),
        "setupManifestInvalid",
    );

    let foreign = state_dir("foreign");
    let rendered = write_manifest(&foreign);
    std::fs::write(
        foreign.join("install-manifest.json"),
        rendered.replace("\"platform\": \"linux\"", "\"platform\": \"macos\""),
    )
    .unwrap();
    let backend = LinuxRuntimeBackend::new(&foreign).unwrap();
    assert_gate(
        &backend.diagnose(&Settings::default()).unwrap(),
        "setupManifestInvalid",
    );

    let healthy = state_dir("healthy");
    write_manifest(&healthy);
    let backend = LinuxRuntimeBackend::new(&healthy).unwrap();
    assert!(backend.diagnose(&Settings::default()).unwrap().is_empty());
}

#[test]
fn skeleton_is_stopped_stoppable_and_unavailable() {
    let directory = state_dir("idle");
    let backend = LinuxRuntimeBackend::new(&directory).unwrap();
    assert!(matches!(
        backend.inspect().unwrap(),
        own_domains_protocol::RuntimeStatus::Stopped
    ));
    backend.stop().unwrap();
    assert!(matches!(
        backend.apply(&Settings::default()),
        Err(RuntimeError::Unavailable(_))
    ));
}

#[test]
#[cfg(target_os = "linux")]
fn facade_selects_the_linux_backend() {
    let directory = state_dir("facade");
    let backend =
        own_domains_daemon::platform::runtime_backend(&directory, None, None, false).unwrap();
    assert_gate(
        &backend.diagnose(&Settings::default()).unwrap(),
        "setupRequired",
    );

    let backend =
        own_domains_daemon::platform::runtime_backend(&directory, None, None, true).unwrap();
    assert!(backend.diagnose(&Settings::default()).unwrap().is_empty());
}
