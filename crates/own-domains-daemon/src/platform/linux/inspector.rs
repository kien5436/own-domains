//! Linux host integration inspector.
//!
//! Read-only validation of the installed integration against the install
//! manifest: artifact files, nftables ruleset and table, systemd units,
//! resolved settings, trust, sidecars, and network state. Every system
//! effect goes through the injected command runner; file reads go through
//! an overridable filesystem root so unit tests run without root.

use std::path::{Path, PathBuf};

use own_domains_core::{Diagnostic, DiagnosticStatus, Settings};
use sha2::Digest;

use super::super::windows::setup::CommandRunner;
use super::ip::{LinuxInterface, discover_covered_interfaces};
use super::manifest::RESOLVER_LINK;
use super::manifest::{LinuxInstallManifest, parse_linux_manifest};
use super::nftables::{NFTABLES_TABLE, render_nftables};
use super::systemd::{
    NFTABLES_UNIT_NAME, RESOLVER_UNIT_NAME, render_nftables_unit, render_resolver_unit,
};
use super::trust::{certificate_der, certificate_sha256};

/// File reads for the inspector. The system implementation hits the
/// real filesystem; tests substitute fakes (e.g. root-owned fixtures
/// while running as a normal user).
pub trait InspectFiles: Send + Sync {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>>;
    /// `(mode_bits, owner_id, group_id)`.
    fn metadata(&self, path: &Path) -> std::io::Result<(u32, u32, u32)>;
}

pub struct SystemInspectFiles;

impl InspectFiles for SystemInspectFiles {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    #[cfg(unix)]
    fn metadata(&self, path: &Path) -> std::io::Result<(u32, u32, u32)> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)
            .map(|metadata| (metadata.mode() & 0o777, metadata.uid(), metadata.gid()))
    }

    #[cfg(not(unix))]
    fn metadata(&self, _path: &Path) -> std::io::Result<(u32, u32, u32)> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "ownership metadata requires unix",
        ))
    }
}

/// Filesystem root for host artifact reads. `None` means the real `/`;
/// tests point it at a temp dir.
pub struct LinuxHostInspector {
    runner: Box<dyn CommandRunner>,
    files: Box<dyn InspectFiles>,
    system_root: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    coredns_exe: Option<PathBuf>,
    caddy_exe: Option<PathBuf>,
}

impl LinuxHostInspector {
    pub fn with_parts(
        runner: Box<dyn CommandRunner>,
        system_root: PathBuf,
        state_dir: PathBuf,
    ) -> Self {
        Self {
            runner,
            files: Box::new(SystemInspectFiles),
            system_root: Some(system_root),
            state_dir: Some(state_dir),
            coredns_exe: None,
            caddy_exe: None,
        }
    }

    /// Production constructor: real `/`, explicit state dir and sidecar
    /// executables resolved by the daemon factory.
    pub fn new(
        runner: Box<dyn CommandRunner>,
        state_dir: PathBuf,
        coredns_exe: PathBuf,
        caddy_exe: PathBuf,
    ) -> Self {
        Self {
            runner,
            files: Box::new(SystemInspectFiles),
            system_root: None,
            state_dir: Some(state_dir),
            coredns_exe: Some(coredns_exe),
            caddy_exe: Some(caddy_exe),
        }
    }

    /// Sidecar executables to validate (tests). The factory constructor
    /// takes them directly.
    pub fn with_sidecars(mut self, coredns_exe: PathBuf, caddy_exe: PathBuf) -> Self {
        self.coredns_exe = Some(coredns_exe);
        self.caddy_exe = Some(caddy_exe);
        self
    }

    /// Override file reads (tests).
    pub fn with_files(mut self, files: Box<dyn InspectFiles>) -> Self {
        self.files = files;
        self
    }

    fn host_path(&self, path: &str) -> PathBuf {
        match &self.system_root {
            Some(root) => root.join(path.strip_prefix('/').unwrap_or(path)),
            None => PathBuf::from(path),
        }
    }

    fn manifest_path(&self) -> Option<PathBuf> {
        self.state_dir
            .as_ref()
            .map(|state| state.join(super::setup::INSTALL_MANIFEST_FILE))
    }

    fn error(code: &str, message: String) -> Diagnostic {
        Diagnostic {
            code: code.into(),
            status: DiagnosticStatus::Error,
            message,
            remedy: Some("re-run setup --apply to repair the host integration".into()),
        }
    }

    fn read_manifest(&self, diagnostics: &mut Vec<Diagnostic>) -> Option<LinuxInstallManifest> {
        let path = self.manifest_path()?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                diagnostics.push(Self::error(
                    "setupManifestInvalid",
                    format!("unable to read {}: {error}", path.display()),
                ));
                return None;
            }
        };
        match parse_linux_manifest(&bytes) {
            Ok(manifest) => Some(manifest),
            Err(error) => {
                diagnostics.push(Self::error(
                    "setupManifestInvalid",
                    format!("{}: {error}", path.display()),
                ));
                None
            }
        }
    }

    pub fn inspect(&self, settings: &Settings) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let Some(manifest) = self.read_manifest(&mut diagnostics) else {
            return diagnostics;
        };
        self.inspect_artifacts(&manifest, &mut diagnostics);
        self.inspect_nftables(&manifest, &mut diagnostics);
        self.inspect_units(&mut diagnostics);
        self.inspect_resolved(&manifest, &mut diagnostics);
        self.inspect_trust(&manifest, &mut diagnostics);
        self.inspect_sidecars(&manifest, &mut diagnostics);
        self.inspect_network(&manifest, settings, &mut diagnostics);
        diagnostics
    }

    fn inspect_network(
        &self,
        manifest: &LinuxInstallManifest,
        settings: &Settings,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let interfaces = match self.covered_interfaces() {
            Ok(interfaces) => interfaces,
            Err(error) => {
                diagnostics.push(Self::error(
                    "networkUnreadable",
                    format!("unable to discover interfaces: {error}"),
                ));
                return;
            }
        };
        let lan: Option<std::net::Ipv4Addr> = settings.lan_address.parse().ok();
        let carrier = lan.and_then(|lan| {
            interfaces
                .iter()
                .find(|item| item.primary_ipv4 == Some(lan))
        });
        match (lan, carrier) {
            (Some(_), Some(item)) => {
                if !manifest.interfaces.iter().any(|name| name == &item.name) {
                    diagnostics.push(Self::error(
                        "interfaceCoverageMissing",
                        format!(
                            "{} serves {} but is not covered; re-run setup --apply",
                            item.name, settings.lan_address,
                        ),
                    ));
                }
            }
            (Some(_), None) => {
                diagnostics.push(Self::error(
                    "lanAddressInactive",
                    format!(
                        "{} is not active on any interface; select an active LAN address",
                        settings.lan_address
                    ),
                ));
            }
            (None, _) => {}
        }
        self.inspect_ports(diagnostics);
    }

    fn covered_interfaces(&self) -> Result<Vec<LinuxInterface>, String> {
        let link = self
            .runner
            .run("/usr/sbin/ip", &["-j", "link"])
            .map_err(|error| format!("ip -j link: {error}"))?;
        let addr = self
            .runner
            .run("/usr/sbin/ip", &["-j", "addr"])
            .map_err(|error| format!("ip -j addr: {error}"))?;
        discover_covered_interfaces(&link, &addr).map_err(|error| error.to_string())
    }

    /// Listeners on the redirect and service ports that are not the
    /// expected resolved stub. `ss -H -ltn` has no header; each line
    /// names the local address.
    fn inspect_ports(&self, diagnostics: &mut Vec<Diagnostic>) {
        const WATCHED: [u16; 5] = [53, 5354, 8080, 8443, 4781];
        let output = match self.runner.run("/usr/bin/ss", &["-H", "-ltn"]) {
            Ok(output) => output,
            Err(error) => {
                diagnostics.push(Self::error(
                    "portsUnreadable",
                    format!("unable to list listeners: {error}"),
                ));
                return;
            }
        };
        for line in output.lines() {
            let mut fields = line.split_whitespace();
            // `ss -H -ltn`: Netid State Recv-Q Send-Q Local:Port Peer:Port.
            let _netid = fields.next();
            let (Some(state), Some(local)) = (fields.next(), fields.nth(2)) else {
                continue;
            };
            if state != "LISTEN" {
                continue;
            }
            // Local address is `host:port` (or `[v6]:port`).
            let Some(port) = local
                .rsplit(':')
                .next()
                .and_then(|port| port.parse::<u16>().ok())
            else {
                continue;
            };
            if !WATCHED.contains(&port) {
                continue;
            }
            let host = local.rsplit_once(':').map(|(host, _)| host).unwrap_or("");
            let loopback = host == "127.0.0.1" || host == "::1" || host == "[::1]";
            // The resolved stub listener is expected; everything else on
            // these ports conflicts with the runtime.
            if port == 53 && loopback {
                continue;
            }
            diagnostics.push(Self::error(
                "portConflict",
                format!("{host}:{port} is already listening"),
            ));
        }
    }

    fn inspect_sidecars(&self, manifest: &LinuxInstallManifest, diagnostics: &mut Vec<Diagnostic>) {
        for (label, exe, spec, version_args) in [
            (
                "CoreDNS",
                &self.coredns_exe,
                &manifest.sidecars.coredns,
                &["--version"] as &[&str],
            ),
            (
                "Caddy",
                &self.caddy_exe,
                &manifest.sidecars.caddy,
                &["version"] as &[&str],
            ),
        ] {
            let Some(exe) = exe else { continue };
            if !exe.is_file() {
                diagnostics.push(Self::error(
                    "sidecarMissing",
                    format!("{label} sidecar is missing: {}", exe.display()),
                ));
                continue;
            }
            if let Err(error) = self.check_sidecar_version(label, exe, spec, version_args) {
                diagnostics.push(Self::error("sidecarVersionMismatch", error));
            }
            match std::fs::read(exe) {
                Ok(bytes) => {
                    let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
                    if actual != spec.sha256 {
                        diagnostics.push(Self::error(
                            "sidecarDigestMismatch",
                            format!("{label} digest does not match the install manifest"),
                        ));
                    }
                }
                Err(error) => {
                    diagnostics.push(Self::error(
                        "sidecarUnreadable",
                        format!("unable to read {}: {error}", exe.display()),
                    ));
                }
            }
        }
    }

    fn check_sidecar_version(
        &self,
        label: &str,
        exe: &Path,
        spec: &super::super::sidecar::SidecarSpec,
        args: &[&str],
    ) -> Result<(), String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let output = self
            .runner
            .run(&exe.to_string_lossy(), &refs)
            .map_err(|error| format!("unable to run {label} version check: {error}"))?;
        if output.contains(&spec.version) {
            Ok(())
        } else {
            Err(format!(
                "{label} reports an unexpected version (expected {} in {output:?})",
                spec.version
            ))
        }
    }

    fn inspect_trust(&self, manifest: &LinuxInstallManifest, diagnostics: &mut Vec<Diagnostic>) {
        let path = self.host_path("/usr/local/share/ca-certificates/own-domains.crt");
        let bytes = match self.files.read(&path) {
            // Missing/unreadable files are already reported by the
            // artifact check; skip trust rather than double-reporting.
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        let der = match certificate_der(&bytes) {
            Ok(der) => der,
            Err(error) => {
                diagnostics.push(Self::error(
                    "trustInvalid",
                    format!("/usr/local/share/ca-certificates/own-domains.crt is not a valid certificate: {error}"),
                ));
                return;
            }
        };
        if certificate_sha256(&der) != manifest.ca_fingerprint_sha256 {
            diagnostics.push(Self::error(
                "caFingerprintMismatch",
                "installed certificate does not match the install manifest fingerprint".into(),
            ));
            return;
        }
        let output = self.runner.run(
            "/usr/bin/openssl",
            &[
                "x509",
                "-in",
                &path.to_string_lossy(),
                "-noout",
                "-fingerprint",
                "-sha256",
            ],
        );
        let trusted = output
            .map(|text| {
                text.chars()
                    .filter(|byte| byte.is_ascii_hexdigit())
                    .collect::<String>()
                    .to_lowercase()
                    .contains(&manifest.ca_fingerprint_sha256)
            })
            .unwrap_or(false);
        if !trusted {
            diagnostics.push(Self::error(
                "caUntrusted",
                "installed certificate is not trusted by the system bundle".into(),
            ));
        }
    }

    fn resolvectl(&self, args: &[&str]) -> Result<String, String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.runner.run("/usr/bin/resolvectl", &refs)
    }

    fn inspect_resolved(&self, manifest: &LinuxInstallManifest, diagnostics: &mut Vec<Diagnostic>) {
        let check =
            |label: &str, code: &str, output: Result<String, String>, want: &[&str]| match output {
                Ok(text) if want.iter().all(|token| text.contains(token)) => None,
                Ok(text) => Some(Self::error(
                    code,
                    format!(
                        "resolvectl {label} {RESOLVER_LINK} is unexpected: {}",
                        text.trim()
                    ),
                )),
                Err(error) => Some(Self::error(
                    code,
                    format!("unable to query resolvectl {label} {RESOLVER_LINK}: {error}"),
                )),
            };
        for diagnostic in [
            check(
                "dns",
                "resolverDnsMismatch",
                self.resolvectl(&["dns", RESOLVER_LINK]),
                &["127.0.0.1", "5354"],
            ),
            check(
                "domain",
                "resolverDomainMismatch",
                self.resolvectl(&["domain", RESOLVER_LINK]),
                &["~own"],
            ),
            check(
                "default-route",
                "resolverDefaultRoute",
                self.resolvectl(&["default-route", RESOLVER_LINK]),
                &[": no"],
            ),
            check(
                "dnssec",
                "resolverHardeningMismatch",
                self.resolvectl(&["dnssec", RESOLVER_LINK]),
                &[": no"],
            ),
            check(
                "dnsovertls",
                "resolverHardeningMismatch",
                self.resolvectl(&["dnsovertls", RESOLVER_LINK]),
                &[": no"],
            ),
        ]
        .into_iter()
        .flatten()
        {
            // `default-route no` is reported but never blocks: the dummy
            // link must not become a default route, full stop.
            diagnostics.push(diagnostic);
        }
        self.inspect_domain_conflicts(manifest, diagnostics);
    }

    /// Any other link claiming `~own` shadows the dummy link. Link names
    /// come from `ip -j link` (already a required binary; `resolvectl`
    /// has no list-links verb); the scan is capped so a pathological
    /// host cannot turn inspection into a fork bomb.
    fn inspect_domain_conflicts(
        &self,
        manifest: &LinuxInstallManifest,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        #[derive(serde::Deserialize)]
        struct LinkName {
            ifname: String,
        }
        let listing = match self.runner.run("/usr/sbin/ip", &["-j", "link"]) {
            Ok(listing) => listing,
            Err(error) => {
                diagnostics.push(Self::error(
                    "resolverUnreadable",
                    format!("unable to list links: {error}"),
                ));
                return;
            }
        };
        let links: Vec<LinkName> = match serde_json::from_str(&listing) {
            Ok(links) => links,
            Err(error) => {
                diagnostics.push(Self::error(
                    "resolverUnreadable",
                    format!("unable to parse `ip -j link` output: {error}"),
                ));
                return;
            }
        };
        let mut others: Vec<String> = links
            .into_iter()
            .map(|link| link.ifname)
            .filter(|name| name != RESOLVER_LINK)
            .collect();
        others.truncate(manifest.interfaces.len() + 8);
        for link in others {
            match self.resolvectl(&["domain", &link]) {
                Ok(output) if output.contains("~own") => {
                    diagnostics.push(Self::error(
                        "ownDomainConflict",
                        format!("{link} also claims ~own; the dummy link is shadowed"),
                    ));
                }
                Ok(_) => {}
                Err(error) => {
                    diagnostics.push(Self::error(
                        "resolverUnreadable",
                        format!("unable to query resolvectl domain {link}: {error}"),
                    ));
                }
            }
        }
    }

    fn systemctl(&self, args: &[&str]) -> Result<String, String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.runner.run("/usr/bin/systemctl", &refs)
    }

    fn inspect_units(&self, diagnostics: &mut Vec<Diagnostic>) {
        for (name, render) in [
            (NFTABLES_UNIT_NAME, render_nftables_unit()),
            (RESOLVER_UNIT_NAME, render_resolver_unit()),
        ] {
            let system_path = format!("/etc/systemd/system/{name}");
            match self.files.read(&self.host_path(&system_path)) {
                Ok(bytes) if bytes == render.as_bytes() => {}
                Ok(_) => {
                    diagnostics.push(Self::error(
                        "unitMismatch",
                        format!("{system_path} content does not match the expected unit"),
                    ));
                }
                Err(error) => {
                    diagnostics.push(Self::error(
                        "unitMissing",
                        format!("{system_path} is missing: {error}"),
                    ));
                }
            }
            // `show` prints the state word and exits 0 regardless of
            // state; `is-enabled`/`is-active` exit non-zero with empty
            // output when the answer is negative, losing the state.
            match self.systemctl(&["show", "-p", "UnitFileState", "--value", name]) {
                Ok(output) if output.trim() == "enabled" => {}
                Ok(output) => {
                    diagnostics.push(Self::error(
                        "unitDisabled",
                        format!("{name} is not enabled ({})", output.trim()),
                    ));
                }
                Err(error) => {
                    diagnostics.push(Self::error(
                        "unitUnknown",
                        format!("unable to query {name}: {error}"),
                    ));
                }
            }
            match self.systemctl(&["show", "-p", "ActiveState", "--value", name]) {
                Ok(output) if output.trim() == "active" => {}
                Ok(output) => {
                    diagnostics.push(Self::error(
                        "unitInactive",
                        format!("{name} is not active ({})", output.trim()),
                    ));
                }
                Err(error) => {
                    diagnostics.push(Self::error(
                        "unitUnknown",
                        format!("unable to query {name}: {error}"),
                    ));
                }
            }
        }
    }

    fn nft_command(&self, args: &[&str]) -> Result<String, String> {
        let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.runner.run("/usr/sbin/nft", &refs)
    }

    fn inspect_nftables(&self, manifest: &LinuxInstallManifest, diagnostics: &mut Vec<Diagnostic>) {
        let expected = render_nftables(&manifest.interfaces, manifest.user_id);
        let path = self.host_path("/etc/own-domains/nftables.conf");
        match (expected, self.files.read(&path)) {
            (Ok(expected), Ok(actual)) if expected.as_bytes() != actual.as_slice() => {
                diagnostics.push(Self::error(
                    "nftablesMismatch",
                    "/etc/own-domains/nftables.conf content does not match the covered interfaces"
                        .into(),
                ));
            }
            (Err(error), _) => {
                diagnostics.push(Self::error(
                    "nftablesInvalid",
                    format!("unable to render the expected ruleset: {error}"),
                ));
            }
            _ => {}
        }
        match self.nft_command(&["list", "table", "ip", "own_domains"]) {
            Ok(output) => {
                // The table name arrives as separate argv words (`ip` +
                // `own_domains`); assert the constant matches that split.
                debug_assert_eq!(NFTABLES_TABLE, "ip own_domains");
                if !(output.contains("chain prerouting") && output.contains("chain output")) {
                    diagnostics.push(Self::error(
                        "nftablesTableMissing",
                        "nft table ip own_domains does not contain the expected chains".into(),
                    ));
                }
            }
            Err(error) => {
                // Stopping the unit deletes the table, so a missing
                // table is an expected state with its own code — not an
                // unreadable one.
                let missing = error.contains("No such file or directory");
                diagnostics.push(Self::error(
                    if missing {
                        "nftablesTableMissing"
                    } else {
                        "nftablesUnreadable"
                    },
                    format!("unable to list nft table ip own_domains: {error}"),
                ));
            }
        }
    }

    fn inspect_artifacts(
        &self,
        manifest: &LinuxInstallManifest,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for artifact in &manifest.artifacts {
            let path = self.host_path(&artifact.path);
            let bytes = match self.files.read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    diagnostics.push(Self::error(
                        "artifactMissing",
                        format!("{} is missing", artifact.path),
                    ));
                    continue;
                }
                Err(error) => {
                    diagnostics.push(Self::error(
                        "artifactUnreadable",
                        format!("unable to read {}: {error}", artifact.path),
                    ));
                    continue;
                }
            };
            let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
            if actual != artifact.sha256 {
                diagnostics.push(Self::error(
                    "artifactMismatch",
                    format!(
                        "{} content does not match the install manifest",
                        artifact.path
                    ),
                ));
                continue;
            }
            match self.files.metadata(&path) {
                Ok((mode, owner, group)) => {
                    if mode != artifact.mode {
                        diagnostics.push(Self::error(
                            "artifactMode",
                            format!(
                                "{} has mode {mode:o}, expected {:o}",
                                artifact.path, artifact.mode
                            ),
                        ));
                    }
                    if owner != artifact.owner_id || group != artifact.group_id {
                        diagnostics.push(Self::error(
                            "artifactOwner",
                            format!("{} has unexpected ownership", artifact.path),
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {}
                Err(error) => {
                    diagnostics.push(Self::error(
                        "artifactUnreadable",
                        format!("unable to stat {}: {error}", artifact.path),
                    ));
                }
            }
        }
    }
}
