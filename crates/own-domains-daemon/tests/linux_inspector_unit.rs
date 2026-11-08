//! Linux host inspector, tested on every platform.
//!
//! All system effects go through the fake runner; file reads go through a
//! temp sysroot. No real mutation here.

use own_domains_core::Settings;
use own_domains_daemon::platform::{
    linux::{
        inspector::{InspectFiles, LinuxHostInspector},
        manifest::{
            LinuxArtifactContent, LinuxArtifactKind, build_linux_manifest, render_linux_manifest,
        },
        nftables::render_nftables,
        systemd::{render_nftables_unit, render_resolver_unit},
    },
    windows::setup::{CommandRunner, RunningProcess},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Responder = Box<dyn Fn(&str, &[String]) -> Result<String, String> + Send>;

struct FakeRunner {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    respond: Mutex<Option<Responder>>,
}

impl FakeRunner {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            respond: Mutex::new(None),
        }
    }

    fn set_responder(
        &self,
        respond: impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static,
    ) {
        *self.respond.lock().unwrap() = Some(Box::new(respond));
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        self.calls
            .lock()
            .unwrap()
            .push((program.to_string(), argv.clone()));
        if let Some(respond) = self.respond.lock().unwrap().as_ref() {
            return respond(program, &argv);
        }
        if program.ends_with("nft") {
            return Ok(
                "table ip own_domains {\n\tchain prerouting {\n\t}\n\n\tchain output {\n\t}\n}"
                    .into(),
            );
        }
        if program.ends_with("resolvectl") {
            let subcommand = args.first().copied().unwrap_or("");
            let link = args.get(1).copied().unwrap_or("");
            if link == "own-domains0" {
                return Ok(match subcommand {
                    "dns" => "Link 4 (own-domains0): 127.0.0.1:5354\n",
                    "domain" => "Link 4 (own-domains0): ~own\n",
                    "default-route" => "Link 4 (own-domains0): no\n",
                    "dnssec" => "Link 4 (own-domains0): no\n",
                    "dnsovertls" => "Link 4 (own-domains0): no\n",
                    _ => "",
                }
                .into());
            }
            return Ok(String::new());
        }
        if program.ends_with("coredns-test") {
            return Ok("CoreDNS-1.14.7\n".into());
        }
        if program.ends_with("caddy-test") {
            return Ok("v2.11.7\n".into());
        }
        if program.ends_with("ip") && args.contains(&"-j") {
            if args.contains(&"link") {
                return Ok(fixture("ip-link.json"));
            }
            if args.contains(&"addr") {
                return Ok(fixture("ip-addr.json"));
            }
        }
        if program.ends_with("ss") {
            return Ok(String::new());
        }
        if program.ends_with("openssl") {
            return Ok("SHA256 Fingerprint=3F:6E:6C:C2:51:61:13:8D:90:67:FF:AD:9E:9E:AA:F5:7B:BB:82:FB:A3:CF:83:C4:EC:37:EF:A9:6C:F3:18:BA\n".into());
        }
        if program.ends_with("systemctl") {
            if args.contains(&"UnitFileState") {
                return Ok("enabled\n".into());
            }
            if args.contains(&"ActiveState") {
                return Ok("active\n".into());
            }
            return Ok(String::new());
        }
        Ok(String::new())
    }

    fn spawn(&self, _program: &str, _args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
        Err("spawn is not used by the inspector".into())
    }
}

struct Tree {
    runner: Arc<FakeRunner>,
    state: PathBuf,
    root: PathBuf,
    coredns_exe: PathBuf,
    caddy_exe: PathBuf,
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("od-linux-inspect-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Healthy tree: the four artifact files under a temp sysroot plus a
/// manifest whose digests match the file bytes.
fn healthy_tree() -> Tree {
    let runner = Arc::new(FakeRunner::new());
    let state = tmp_dir("state");
    let root = tmp_dir("sysroot");
    let ruleset = render_nftables(&["enp3s0"], 1000).unwrap();
    let nftables_unit = render_nftables_unit();
    let resolver_unit = render_resolver_unit();
    // Minimal DER SEQUENCE so the trust check can parse it.
    let der = [0x30u8, 0x03, 0x01, 0x02, 0x03];
    let files: [(LinuxArtifactKind, &str, &[u8]); 4] = [
        (
            LinuxArtifactKind::Nftables,
            "/etc/own-domains/nftables.conf",
            ruleset.as_bytes(),
        ),
        (
            LinuxArtifactKind::SystemdUnit,
            "/etc/systemd/system/own-domains-nftables.service",
            nftables_unit.as_bytes(),
        ),
        (
            LinuxArtifactKind::SystemdUnit,
            "/etc/systemd/system/own-domains-resolver.service",
            resolver_unit.as_bytes(),
        ),
        (
            LinuxArtifactKind::TrustCertificate,
            "/usr/local/share/ca-certificates/own-domains.crt",
            der.as_slice(),
        ),
    ];
    for (_, path, bytes) in &files {
        let full = root.join(path.strip_prefix('/').unwrap());
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
    }
    let contents: Vec<LinuxArtifactContent<'_>> = files
        .iter()
        .map(|(kind, path, bytes)| LinuxArtifactContent {
            path,
            kind: *kind,
            content: bytes,
        })
        .collect();
    let coredns_exe = state.join("coredns-test");
    let caddy_exe = state.join("caddy-test");
    std::fs::write(&coredns_exe, b"fake-coredns-binary").unwrap();
    std::fs::write(&caddy_exe, b"fake-caddy-binary").unwrap();
    use sha2::Digest;
    let sidecars = own_domains_daemon::platform::SidecarManifest {
        schema_version: 1,
        coredns: own_domains_daemon::platform::SidecarSpec {
            version: "1.14.7".into(),
            sha256: format!(
                "{:x}",
                sha2::Sha256::digest(std::fs::read(&coredns_exe).unwrap())
            ),
        },
        caddy: own_domains_daemon::platform::SidecarSpec {
            version: "2.11.7".into(),
            sha256: format!(
                "{:x}",
                sha2::Sha256::digest(std::fs::read(&caddy_exe).unwrap())
            ),
        },
    };
    let manifest = build_linux_manifest(
        "testuser",
        1000,
        &["enp3s0"],
        sidecars,
        &own_domains_daemon::platform::linux::trust::certificate_sha256(&der),
        &contents,
    );
    let text = render_linux_manifest(&manifest).unwrap();
    std::fs::write(state.join("install-manifest.json"), text).unwrap();
    Tree {
        runner,
        state,
        root,
        coredns_exe,
        caddy_exe,
    }
}

struct FakeFiles;

impl InspectFiles for FakeFiles {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn metadata(&self, _path: &Path) -> std::io::Result<(u32, u32, u32)> {
        // Fixtures are user-owned in temp dirs; report the production
        // expectation (root-owned 0644) so ownership checks pass.
        Ok((0o644, 0, 0))
    }
}

fn inspector(tree: &Tree) -> LinuxHostInspector {
    LinuxHostInspector::with_parts(
        Box::new(Handle(tree.runner.clone())),
        tree.root.clone(),
        tree.state.clone(),
    )
    .with_files(Box::new(FakeFiles))
    .with_sidecars(tree.coredns_exe.clone(), tree.caddy_exe.clone())
}

struct Handle(Arc<FakeRunner>);

impl CommandRunner for Handle {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        self.0.run(program, args)
    }

    fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
        self.0.spawn(program, args)
    }
}
fn settings() -> Settings {
    Settings {
        lan_address: "192.168.1.20".into(),
        ..Settings::default()
    }
}

#[test]
fn healthy_tree_reports_no_diagnostics() {
    let tree = healthy_tree();
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn digest_mismatch_names_the_path() {
    let tree = healthy_tree();
    std::fs::write(tree.root.join("etc/own-domains/nftables.conf"), b"tampered").unwrap();

    let diagnostics = inspector(&tree).inspect(&settings());
    let codes: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains(&"artifactMismatch"), "{diagnostics:?}");
    // The tampered file also drifts from the rendered ruleset.
    assert!(codes.contains(&"nftablesMismatch"), "{diagnostics:?}");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("/etc/own-domains/nftables.conf")),
        "{diagnostics:?}"
    );
}

#[test]
fn missing_file_reports_artifact_missing() {
    let tree = healthy_tree();
    std::fs::remove_file(tree.root.join("etc/own-domains/nftables.conf")).unwrap();

    let diagnostics = inspector(&tree).inspect(&settings());
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "artifactMissing");
}

#[test]
fn settings_without_lan_address_is_not_a_failure() {
    let tree = healthy_tree();
    let diagnostics = inspector(&tree).inspect(&Settings::default());
    // Address activity is Task 7; the file checks must not depend on it.
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

fn nft_output(
    list_output: &str,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let list_output = list_output.to_string();
    move |program: &str, _args: &[String]| {
        if program.ends_with("nft") {
            return Ok(list_output.clone());
        }
        Ok(String::new())
    }
}

#[test]
fn nftables_table_missing_is_reported() {
    let tree = healthy_tree();
    tree.runner
        .set_responder(nft_output("table ip filter {\n}"));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "nftablesTableMissing"),
        "{diagnostics:?}"
    );
}

#[test]
fn nftables_unreadable_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(|program, _args| {
        if program.ends_with("nft") {
            return Err("nft: command not found".into());
        }
        Ok(String::new())
    });
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "nftablesUnreadable"),
        "{diagnostics:?}"
    );
}

fn systemctl_output(
    enabled: &str,
    active: &str,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let enabled = enabled.to_string();
    let active = active.to_string();
    move |program: &str, args: &[String]| {
        if program.ends_with("systemctl") {
            if args.contains(&"UnitFileState".to_string()) {
                return Ok(enabled.clone());
            }
            if args.contains(&"ActiveState".to_string()) {
                return Ok(active.clone());
            }
            return Ok(String::new());
        }
        if program.ends_with("nft") {
            return Ok(
                "table ip own_domains {\n\tchain prerouting {\n\t}\n\n\tchain output {\n\t}\n}"
                    .into(),
            );
        }
        Ok(String::new())
    }
}

#[test]
fn unit_content_mismatch_is_reported() {
    let tree = healthy_tree();
    std::fs::write(
        tree.root
            .join("etc/systemd/system/own-domains-nftables.service"),
        b"[Unit]\n# tampered\n",
    )
    .unwrap();
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unitMismatch"
                && diagnostic.message.contains("own-domains-nftables.service")),
        "{diagnostics:?}"
    );
}

#[test]
fn disabled_unit_is_reported() {
    let tree = healthy_tree();
    tree.runner
        .set_responder(systemctl_output("disabled\n", "active\n"));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unitDisabled"),
        "{diagnostics:?}"
    );
}

#[test]
fn inactive_unit_is_reported() {
    let tree = healthy_tree();
    tree.runner
        .set_responder(systemctl_output("enabled\n", "inactive\n"));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unitInactive"),
        "{diagnostics:?}"
    );
}

fn resolved_output(
    dns: &str,
    domain: &str,
    default_route: &str,
    dnssec: &str,
    tls: &str,
    other_domains: &[(&str, &str)],
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let dns = dns.to_string();
    let domain = domain.to_string();
    let default_route = default_route.to_string();
    let dnssec = dnssec.to_string();
    let tls = tls.to_string();
    let other: Vec<(String, String)> = other_domains
        .iter()
        .map(|(link, output)| (link.to_string(), output.to_string()))
        .collect();
    move |program: &str, args: &[String]| {
        if program.ends_with("nft") {
            return Ok(
                "table ip own_domains {\n\tchain prerouting {\n\t}\n\n\tchain output {\n\t}\n}"
                    .into(),
            );
        }
        if program.ends_with("ip") && args.contains(&"-j".to_string()) {
            if args.contains(&"link".to_string()) {
                return Ok(fixture("ip-link.json"));
            }
            if args.contains(&"addr".to_string()) {
                return Ok(fixture("ip-addr.json"));
            }
        }
        if program.ends_with("ss") {
            return Ok(String::new());
        }
        if program.ends_with("coredns-test") {
            return Ok("CoreDNS-1.14.7\n".into());
        }
        if program.ends_with("caddy-test") {
            return Ok("v2.11.7\n".into());
        }
        if program.ends_with("openssl") {
            return Ok("SHA256 Fingerprint=3F:6E:6C:C2:51:61:13:8D:90:67:FF:AD:9E:9E:AA:F5:7B:BB:82:FB:A3:CF:83:C4:EC:37:EF:A9:6C:F3:18:BA\n".into());
        }
        if program.ends_with("systemctl") {
            if args.contains(&"UnitFileState".to_string()) {
                return Ok("enabled\n".into());
            }
            if args.contains(&"ActiveState".to_string()) {
                return Ok("active\n".into());
            }
            return Ok(String::new());
        }
        if program.ends_with("resolvectl") {
            let subcommand = args.first().map(String::as_str).unwrap_or("");
            let link = args.get(1).map(String::as_str).unwrap_or("");
            if link == "own-domains0" {
                return Ok(match subcommand {
                    "dns" => dns.clone(),
                    "domain" => domain.clone(),
                    "default-route" => default_route.clone(),
                    "dnssec" => dnssec.clone(),
                    "dnsovertls" => tls.clone(),
                    _ => String::new(),
                });
            }
            if subcommand == "domain"
                && let Some((_, output)) = other.iter().find(|(name, _)| name == link)
            {
                return Ok(output.clone());
            }
            return Ok(String::new());
        }
        Ok(String::new())
    }
}

#[test]
fn resolved_dns_mismatch_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(resolved_output(
        "Link 4 (own-domains0): 8.8.8.8\n",
        "Link 4 (own-domains0): ~own\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        &[],
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "resolverDnsMismatch"),
        "{diagnostics:?}"
    );
}

#[test]
fn resolved_domain_mismatch_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(resolved_output(
        "Link 4 (own-domains0): 127.0.0.1:5354\n",
        "Link 4 (own-domains0): example.com\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        &[],
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "resolverDomainMismatch"),
        "{diagnostics:?}"
    );
}

#[test]
fn resolved_default_route_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(resolved_output(
        "Link 4 (own-domains0): 127.0.0.1:5354\n",
        "Link 4 (own-domains0): ~own\n",
        "Link 4 (own-domains0): yes\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        &[],
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "resolverDefaultRoute"),
        "{diagnostics:?}"
    );
}

#[test]
fn resolved_hardening_mismatch_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(resolved_output(
        "Link 4 (own-domains0): 127.0.0.1:5354\n",
        "Link 4 (own-domains0): ~own\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): yes\n",
        "Link 4 (own-domains0): opportunistic\n",
        &[],
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "resolverHardeningMismatch"),
        "{diagnostics:?}"
    );
}

#[test]
fn competing_own_claim_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(resolved_output(
        "Link 4 (own-domains0): 127.0.0.1:5354\n",
        "Link 4 (own-domains0): ~own\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        &[("enp3s0", "Link 3 (enp3s0): ~own\n")],
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "ownDomainConflict"
                && diagnostic.message.contains("enp3s0")),
        "{diagnostics:?}"
    );
}

#[test]
fn healthy_resolved_tree_is_silent() {
    let tree = healthy_tree();
    tree.runner.set_responder(resolved_output(
        "Link 4 (own-domains0): 127.0.0.1:5354\n",
        "Link 4 (own-domains0): ~own\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        "Link 4 (own-domains0): no\n",
        &[],
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn trust_fingerprint_mismatch_is_reported() {
    let tree = healthy_tree();
    let manifest_path = tree.state.join("install-manifest.json");
    let text = std::fs::read_to_string(&manifest_path).unwrap();
    let tampered = text.replace(
        "3f6e6cc25161138d9067ffad9e9eaaf57bbb82fba3cf83c4ec37efa96cf318ba",
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    );
    assert_ne!(text, tampered, "fixture fingerprint changed?");
    std::fs::write(&manifest_path, tampered).unwrap();

    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "caFingerprintMismatch"),
        "{diagnostics:?}"
    );
}

#[test]
fn untrusted_cert_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(|program, args| {
        if program.ends_with("nft") {
            return Ok("table ip own_domains {\n\tchain prerouting {\n\t}\n\n\tchain output {\n\t}\n}".into());
        }
        if program.ends_with("systemctl") {
            return Ok("enabled\n".into());
        }
        if program.ends_with("resolvectl") {
            if args.get(1).map(String::as_str) == Some("own-domains0") {
                return Ok("Link 4 (own-domains0): 127.0.0.1:5354\nLink 4 (own-domains0): ~own\nLink 4 (own-domains0): no\n".into());
            }
            return Ok(String::new());
        }
        if program.ends_with("coredns-test") {
            return Ok("CoreDNS-1.14.7\n".into());
        }
        if program.ends_with("caddy-test") {
            return Ok("v2.11.7\n".into());
        }
        if program.ends_with("openssl") {
            return Ok("SHA256 Fingerprint=00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00\n".into());
        }
        Ok(String::new())
    });
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "caUntrusted"),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "caFingerprintMismatch"),
        "file fingerprint still matches: {diagnostics:?}"
    );
}

#[test]
fn missing_sidecars_are_reported() {
    let tree = healthy_tree();
    std::fs::remove_file(&tree.coredns_exe).unwrap();
    std::fs::remove_file(&tree.caddy_exe).unwrap();
    let inspector = LinuxHostInspector::with_parts(
        Box::new(Handle(tree.runner.clone())),
        tree.root.clone(),
        tree.state.clone(),
    )
    .with_files(Box::new(FakeFiles))
    .with_sidecars(tree.coredns_exe.clone(), tree.caddy_exe.clone());

    let diagnostics = inspector.inspect(&settings());
    let missing: Vec<&str> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sidecarMissing")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(missing.len(), 2, "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "sidecarVersionMismatch"),
        "version must not be probed when the binary is missing: {diagnostics:?}"
    );
}

#[test]
fn sidecar_version_mismatch_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(|program, args| {
        if program.ends_with("caddy-test") {
            return Ok("v9.9.9\n".into());
        }
        let _ = args;
        Ok(String::new())
    });
    // NOTE: this blanket responder shadows the healthy defaults; the nft
    // table listing is empty so nftablesUnreadable also fires — assert
    // only the sidecar code.
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "sidecarVersionMismatch"
                && diagnostic.message.contains("Caddy")),
        "{diagnostics:?}"
    );
}

#[test]
fn sidecar_digest_mismatch_is_reported() {
    let tree = healthy_tree();
    std::fs::write(&tree.caddy_exe, b"tampered-caddy").unwrap();
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "sidecarDigestMismatch"
                && diagnostic.message.contains("Caddy")),
        "{diagnostics:?}"
    );
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/linux")
            .join(name),
    )
    .unwrap()
}

fn network_responder(
    ss: &str,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let link = fixture("ip-link.json");
    let addr = fixture("ip-addr.json");
    let ss = ss.to_string();
    move |program: &str, args: &[String]| {
        if program.ends_with("ip") && args.contains(&"-j".to_string()) {
            if args.contains(&"link".to_string()) {
                return Ok(link.clone());
            }
            if args.contains(&"addr".to_string()) {
                return Ok(addr.clone());
            }
        }
        if program.ends_with("ss") {
            return Ok(ss.clone());
        }
        if program.ends_with("nft") {
            return Ok(
                "table ip own_domains {\n\tchain prerouting {\n\t}\n\n\tchain output {\n\t}\n}"
                    .into(),
            );
        }
        if program.ends_with("coredns-test") {
            return Ok("CoreDNS-1.14.7\n".into());
        }
        if program.ends_with("caddy-test") {
            return Ok("v2.11.7\n".into());
        }
        if program.ends_with("openssl") {
            return Ok("SHA256 Fingerprint=3F:6E:6C:C2:51:61:13:8D:90:67:FF:AD:9E:9E:AA:F5:7B:BB:82:FB:A3:CF:83:C4:EC:37:EF:A9:6C:F3:18:BA\n".into());
        }
        if program.ends_with("systemctl") {
            if args.contains(&"UnitFileState".to_string()) {
                return Ok("enabled\n".into());
            }
            if args.contains(&"ActiveState".to_string()) {
                return Ok("active\n".into());
            }
            return Ok(String::new());
        }
        if program.ends_with("resolvectl") {
            if args.get(1).map(String::as_str) == Some("own-domains0") {
                return Ok("Link 4 (own-domains0): 127.0.0.1:5354\nLink 4 (own-domains0): ~own\nLink 4 (own-domains0): no\n".into());
            }
            return Ok(String::new());
        }
        Ok(String::new())
    }
}

#[test]
fn inactive_lan_address_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(network_responder(""));
    let mut offline = settings();
    offline.lan_address = "10.99.99.99".into();

    let diagnostics = inspector(&tree).inspect(&offline);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "lanAddressInactive"),
        "{diagnostics:?}"
    );
}

#[test]
fn uncovered_interface_is_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(network_responder(""));
    // 10.0.0.5 is br0's primary in the fixtures; br0 is not covered.
    let mut roaming = settings();
    roaming.lan_address = "10.0.0.5".into();

    let diagnostics = inspector(&tree).inspect(&roaming);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "interfaceCoverageMissing"
                && diagnostic.message.contains("br0")),
        "{diagnostics:?}"
    );
}

#[test]
fn wildcard_listener_conflicts_but_stub_does_not() {
    let tree = healthy_tree();
    tree.runner.set_responder(network_responder(
        "tcp LISTEN 0 128 127.0.0.1:53 0.0.0.0:*\ntcp LISTEN 0 128 0.0.0.0:8080 0.0.0.0:*\n",
    ));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "portConflict"
                && diagnostic.message.contains("8080")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains(":53")),
        "resolved stub must be ignored: {diagnostics:?}"
    );
}

#[test]
fn stopped_unit_reports_inactive_not_unknown() {
    // Real `systemctl show` exits 0 with the state word even for a
    // stopped unit (unlike `is-active`, which exits non-zero with empty
    // stderr and loses the state).
    let tree = healthy_tree();
    tree.runner
        .set_responder(systemctl_output("enabled\n", "inactive\n"));
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unitInactive"
                && diagnostic.message.contains("own-domains-nftables")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "unitUnknown"),
        "{diagnostics:?}"
    );
}

#[test]
fn deleted_table_reports_missing_not_unreadable() {
    // Stopping the unit deletes the table, so `nft list` fails with
    // "No such file or directory" — that is a missing table, not an
    // unreadable one.
    let tree = healthy_tree();
    tree.runner.set_responder(|program, _args| {
        if program.ends_with("nft") {
            return Err("/usr/sbin/nft failed: Error: No such file or directory\nlist table ip own_domains\n              ^^^^^^^^^^^".into());
        }
        Ok(String::new())
    });
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "nftablesTableMissing"),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "nftablesUnreadable"),
        "{diagnostics:?}"
    );
}

fn firewall_responder(
    ufw: Result<&str, &str>,
    firewalld_state: Result<&str, &str>,
) -> impl Fn(&str, &[String]) -> Result<String, String> + Send + 'static {
    let ufw = ufw.map(str::to_string).map_err(str::to_string);
    let firewalld_state = firewalld_state.map(str::to_string).map_err(str::to_string);
    move |program: &str, args: &[String]| {
        if program.ends_with("ufw") {
            return ufw.clone();
        }
        if program.ends_with("firewall-cmd") {
            if args.contains(&"--state".to_string()) {
                return firewalld_state.clone();
            }
            return Ok("public\n".into());
        }
        Ok(String::new())
    }
}

#[test]
fn blocked_ufw_ports_are_reported() {
    let tree = healthy_tree();
    tree.runner.set_responder(|program, args| {
        firewall_responder(
            Ok("Status: active\n\nTo Action From\n5354/tcp ALLOW Anywhere\n\nDefault: deny (incoming), allow (outgoing)\n"),
            Err("not running"),
        )(program, args)
    });
    let diagnostics = inspector(&tree).inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "firewallBlocked"),
        "{diagnostics:?}"
    );
}

#[test]
fn unreadable_firewall_is_a_warning() {
    let tree = healthy_tree();
    tree.runner.set_responder(|program, args| {
        firewall_responder(
            Err("ufw: command not found"),
            Err("firewall-cmd: command not found"),
        )(program, args)
    });
    let diagnostics = inspector(&tree).inspect(&settings());
    let found: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "firewallUnverifiable")
        .collect();
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics.iter().all(|diagnostic| diagnostic.status
            == own_domains_core::DiagnosticStatus::Warning
            || diagnostic.code != "firewallUnverifiable"),
        "{diagnostics:?}"
    );
}

#[test]
fn flush_ruleset_reports_service_conflict_warning() {
    let tree = healthy_tree();
    let inspector = LinuxHostInspector::with_parts(
        Box::new(Handle(tree.runner.clone())),
        tree.root.clone(),
        tree.state.clone(),
    )
    .with_files(Box::new(FakeFiles))
    .with_sidecars(tree.coredns_exe.clone(), tree.caddy_exe.clone());
    std::fs::create_dir_all(tree.root.join("etc")).unwrap();
    std::fs::write(
        tree.root.join("etc/nftables.conf"),
        "flush ruleset\ntable ip filter {}\n",
    )
    .unwrap();
    tree.runner.set_responder(|program, args| {
        if program.ends_with("systemctl") {
            if args.contains(&"UnitFileState".to_string()) {
                return Ok("enabled\n".into());
            }
            if args.contains(&"ActiveState".to_string()) {
                return Ok("active\n".into());
            }
            return Ok(String::new());
        }
        if program.ends_with("ip") && args.contains(&"-j".to_string()) {
            if args.contains(&"link".to_string()) {
                return Ok(fixture("ip-link.json"));
            }
            if args.contains(&"addr".to_string()) {
                return Ok(fixture("ip-addr.json"));
            }
        }
        Ok(String::new())
    });

    // NOTE: this blanket responder shadows the healthy defaults (nft table
    // listing, openssl, sidecar versions), so only the warning is asserted.
    let diagnostics = inspector.inspect(&settings());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "nftablesServiceConflict"
                && diagnostic.status == own_domains_core::DiagnosticStatus::Warning),
        "{diagnostics:?}"
    );
}
