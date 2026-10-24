use super::{MacOsArchitecture, RuntimePaths};
use own_domains_core::{Diagnostic, DiagnosticStatus, Settings};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};

const ANCHOR_PATH: &str = "/etc/pf.anchors/dev.owndomains";
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_COMMAND_BYTES: usize = 64 * 1024;
const RESOLVER_CONTENT: &str = "nameserver 127.0.0.1\nport 5354\n";
const PF_CONF_BLOCK: &str = concat!(
    "# BEGIN OWN DOMAINS\n",
    "rdr-anchor \"dev.owndomains\"\n",
    "load anchor \"dev.owndomains\" from \"/etc/pf.anchors/dev.owndomains\"\n",
    "# END OWN DOMAINS\n",
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarSpec {
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarManifest {
    pub schema_version: u32,
    pub coredns: SidecarSpec,
    pub caddy: SidecarSpec,
}

#[derive(Clone, Debug)]
pub struct MacOsSetupOptions {
    pub user_name: String,
    pub user_id: u32,
    pub home_directory: PathBuf,
    pub state_directory: PathBuf,
    pub resource_directory: PathBuf,
    pub backup_directory: PathBuf,
    pub lan_address: Ipv4Addr,
    pub interface: String,
    pub covered_interfaces: Vec<NetworkInterface>,
    pub architecture: MacOsArchitecture,
    pub sidecars: SidecarManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkInterface {
    pub name: String,
    pub primary_ipv4: Option<Ipv4Addr>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostArtifact {
    pub path: PathBuf,
    pub content: String,
    pub mode: u32,
    pub owner_id: u32,
    pub backup_existing: bool,
    pub managed_block: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostFileKind {
    Regular,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFileMetadata {
    pub kind: HostFileKind,
    pub owner_id: u32,
    pub mode: u32,
}

pub trait HostFileSystem: Send + Sync {
    fn metadata(&self, path: &Path) -> Result<Option<HostFileMetadata>, String>;
    fn read(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, String>;
    fn sha256(&self, path: &Path, max_bytes: u64) -> Result<String, String>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestArtifact {
    pub path: PathBuf,
    pub sha256: String,
    pub mode: u32,
    pub owner_id: u32,
    pub managed_block: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInstallManifest {
    pub schema_version: u32,
    pub user_name: String,
    pub user_id: u32,
    pub backup_directory: PathBuf,
    pub interfaces: Vec<String>,
    pub artifacts: Vec<ManifestArtifact>,
    pub sidecars: SidecarManifest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyHostInstallManifest {
    pub user_id: u32,
    pub backup_directory: PathBuf,
    pub artifacts: Vec<ManifestArtifact>,
    pub sidecars: SidecarManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstalledHostManifest {
    V1(LegacyHostInstallManifest),
    V2(HostInstallManifest),
}

impl InstalledHostManifest {
    pub(crate) fn user_id(&self) -> u32 {
        match self {
            Self::V1(manifest) => manifest.user_id,
            Self::V2(manifest) => manifest.user_id,
        }
    }

    pub(crate) fn backup_directory(&self) -> &Path {
        match self {
            Self::V1(manifest) => &manifest.backup_directory,
            Self::V2(manifest) => &manifest.backup_directory,
        }
    }

    pub(crate) fn artifacts(&self) -> &[ManifestArtifact] {
        match self {
            Self::V1(manifest) => &manifest.artifacts,
            Self::V2(manifest) => &manifest.artifacts,
        }
    }

    #[allow(dead_code)] // Read by the inspector once schema 2 diagnostics land.
    pub(crate) fn sidecars(&self) -> &SidecarManifest {
        match self {
            Self::V1(manifest) => &manifest.sidecars,
            Self::V2(manifest) => &manifest.sidecars,
        }
    }
}

pub(crate) fn parse_host_install_manifest(bytes: &[u8]) -> Result<InstalledHostManifest, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Header {
        schema_version: u32,
    }

    let header: Header = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    match header.schema_version {
        1 => serde_json::from_slice::<LegacyHostInstallManifest>(bytes)
            .map(InstalledHostManifest::V1)
            .map_err(|error| error.to_string()),
        2 => serde_json::from_slice::<HostInstallManifest>(bytes)
            .map(InstalledHostManifest::V2)
            .map_err(|error| error.to_string()),
        version => Err(format!("unsupported install manifest schema {version}")),
    }
}

pub(crate) fn validate_installed_manifest(
    paths: &RuntimePaths,
    manifest: &InstalledHostManifest,
) -> Result<(), String> {
    let backup = manifest.backup_directory();
    if !backup.is_absolute()
        || !backup.starts_with(&paths.state_directory)
        || backup
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err("install manifest backup directory is invalid".into());
    }
    let artifacts = manifest.artifacts();
    if artifacts.len() != 3 {
        return Err("install manifest artifact list is invalid".into());
    }
    for (path, managed) in [
        (Path::new("/etc/resolver/own"), false),
        (Path::new(ANCHOR_PATH), false),
        (Path::new("/etc/pf.conf"), true),
    ] {
        let Some(artifact) = artifacts.iter().find(|artifact| artifact.path == path) else {
            return Err(format!(
                "install manifest artifact list is invalid: missing {}",
                path.display()
            ));
        };
        if artifact.managed_block != managed
            || artifact.mode != 0o644
            || artifact.owner_id != 0
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "install manifest artifact is invalid: {}",
                path.display()
            ));
        }
    }
    if let InstalledHostManifest::V2(manifest) = manifest {
        let mut names = HashSet::new();
        if manifest.schema_version != 2
            || manifest.interfaces.is_empty()
            || !manifest
                .interfaces
                .iter()
                .all(|name| is_eligible_interface(name) && names.insert(name.as_str()))
        {
            return Err("install manifest interface list is invalid".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct HostSetupPlan {
    pub paths: RuntimePaths,
    pub artifacts: Vec<HostArtifact>,
    pub commands: Vec<HostCommand>,
    pub manifest: HostInstallManifest,
}

impl HostSetupPlan {
    pub fn artifact(&self, path: impl AsRef<Path>) -> Option<&HostArtifact> {
        let path = path.as_ref();
        self.artifacts.iter().find(|artifact| artifact.path == path)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostSetupError {
    #[error("{label} must be an absolute path: {path}")]
    RelativePath { label: &'static str, path: PathBuf },
    #[error("invalid network interface: {0}")]
    InvalidInterface(String),
    #[error("no eligible network interfaces were found")]
    EmptyInterfaceCoverage,
    #[error("duplicate network interface: {0}")]
    DuplicateInterface(String),
    #[error("selected interface is not covered: {0}")]
    InterfaceNotCovered(String),
    #[error("LAN address {address} is not the primary IPv4 address of interface {interface}")]
    AddressMismatch {
        interface: String,
        address: Ipv4Addr,
    },
    #[error("invalid sidecar manifest schema: {0}")]
    InvalidManifestSchema(u32),
    #[error("invalid {sidecar} version")]
    InvalidVersion { sidecar: &'static str },
    #[error("invalid {sidecar} SHA-256 digest")]
    InvalidDigest { sidecar: &'static str },
    #[error("duplicate setup artifact: {0}")]
    DuplicateArtifact(PathBuf),
    #[error("setup artifact path must be absolute: {0}")]
    RelativeArtifact(PathBuf),
    #[error("{label} is not representable as UTF-8: {path}")]
    NonUtf8Path { label: &'static str, path: PathBuf },
    #[error("invalid target user: {0}")]
    InvalidUser(String),
    #[error("state and backup directories must remain inside the target user's home")]
    InvalidDirectoryLayout,
    #[error("failed to serialize install manifest: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn discover_network_interfaces(
    commands: &dyn CommandRunner,
) -> Result<Vec<NetworkInterface>, String> {
    let list = commands.run(&inspect_command("/sbin/ifconfig", ["-l"]))?;
    if list.status != 0 {
        return Err(format!(
            "ifconfig -l failed: {}",
            command_detail(&list.stderr)
        ));
    }
    let mut interfaces = Vec::new();
    for name in String::from_utf8_lossy(&list.stdout).split_ascii_whitespace() {
        if !is_eligible_interface(name) {
            continue;
        }
        let output = commands.run(&inspect_command("/sbin/ifconfig", [name]))?;
        if output.status != 0 {
            return Err(format!(
                "ifconfig {name} failed: {}",
                command_detail(&output.stderr)
            ));
        }
        interfaces.push(NetworkInterface {
            name: name.into(),
            primary_ipv4: parse_primary_ipv4(&String::from_utf8_lossy(&output.stdout)),
        });
    }
    interfaces.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(interfaces)
}

fn is_eligible_interface(name: &str) -> bool {
    (name.starts_with("en") || name.starts_with("bridge"))
        && name.len() <= 15
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_primary_ipv4(output: &str) -> Option<Ipv4Addr> {
    for line in output.lines() {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        // macOS prints the primary IPv4 address first and lists aliases as
        // subsequent `inet` lines. PF's `(<interface>:0)` selects that first
        // address, so return the first `inet` address without alias filtering.
        if let Some(index) = fields.iter().position(|field| *field == "inet") {
            return fields.get(index + 1).and_then(|value| value.parse().ok());
        }
    }
    None
}

fn command_detail(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(240)
        .collect()
}

pub struct HostIntegrationInspector<'a> {
    paths: &'a RuntimePaths,
    sidecars: &'a SidecarManifest,
    files: &'a dyn HostFileSystem,
    commands: &'a dyn CommandRunner,
}

impl<'a> HostIntegrationInspector<'a> {
    pub fn new(
        paths: &'a RuntimePaths,
        sidecars: &'a SidecarManifest,
        files: &'a dyn HostFileSystem,
        commands: &'a dyn CommandRunner,
    ) -> Self {
        Self {
            paths,
            sidecars,
            files,
            commands,
        }
    }

    pub fn inspect(&self, _settings: &Settings) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let manifest = match self.read_manifest(&mut diagnostics) {
            Some(manifest) => manifest,
            None => return diagnostics,
        };
        self.inspect_managed_file(
            Path::new("/etc/resolver/own"),
            0,
            Some(RESOLVER_CONTENT),
            "resolverMismatch",
            &mut diagnostics,
        );
        let expected_anchor = render_anchor_for(&manifest.interfaces);
        self.inspect_managed_file(
            Path::new(ANCHOR_PATH),
            0,
            Some(&expected_anchor),
            "pfAnchorInvalid",
            &mut diagnostics,
        );
        self.inspect_pf_conf(&mut diagnostics);
        self.inspect_pf(&mut diagnostics);
        self.inspect_trust(&mut diagnostics);
        self.inspect_sidecar(
            "CoreDNS",
            &self.paths.coredns_binary,
            &self.sidecars.coredns,
            ["--version"],
            &mut diagnostics,
        );
        self.inspect_sidecar(
            "Caddy",
            &self.paths.caddy_binary,
            &self.sidecars.caddy,
            ["version"],
            &mut diagnostics,
        );
        self.inspect_ports(&mut diagnostics);
        diagnostics
    }

    fn read_manifest(&self, diagnostics: &mut Vec<Diagnostic>) -> Option<HostInstallManifest> {
        let metadata = match self.files.metadata(&self.paths.install_manifest) {
            Ok(Some(metadata)) if metadata.kind == HostFileKind::Regular => metadata,
            Ok(Some(_)) => {
                diagnostics.push(error(
                    "setupManifestInvalid",
                    "The setup manifest is not a regular file.",
                    "Run privileged setup again.",
                ));
                return None;
            }
            Ok(None) => {
                diagnostics.push(error(
                    "setupMissing",
                    "Own Domains host setup is missing.",
                    "Run privileged setup.",
                ));
                return None;
            }
            Err(message) => {
                diagnostics.push(error(
                    "setupManifestInvalid",
                    &format!("Unable to inspect the setup manifest: {message}"),
                    "Check state-directory permissions, then rerun setup.",
                ));
                return None;
            }
        };
        let bytes = match self
            .files
            .read(&self.paths.install_manifest, MAX_FILE_BYTES)
        {
            Ok(bytes) => bytes,
            Err(message) => {
                diagnostics.push(error(
                    "setupManifestInvalid",
                    &format!("Unable to read the setup manifest: {message}"),
                    "Run privileged setup again.",
                ));
                return None;
            }
        };
        match serde_json::from_slice::<HostInstallManifest>(&bytes) {
            Ok(manifest)
                if manifest.schema_version == 2
                    && &manifest.sidecars == self.sidecars
                    && metadata.owner_id == manifest.user_id
                    && metadata.mode == 0o600 =>
            {
                Some(manifest)
            }
            _ => {
                diagnostics.push(error(
                    "setupManifestIncompatible",
                    "The setup manifest is incompatible with this app build.",
                    "Run privileged setup again.",
                ));
                None
            }
        }
    }

    fn inspect_managed_file(
        &self,
        path: &Path,
        owner: u32,
        exact: Option<&str>,
        code: &'static str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let valid = matches!(
            self.files.metadata(path),
            Ok(Some(HostFileMetadata { kind: HostFileKind::Regular, owner_id, .. })) if owner_id == owner
        ) && exact.is_none_or(|content| {
            self.files
                .read(path, MAX_FILE_BYTES)
                .is_ok_and(|bytes| bytes == content.as_bytes())
        });
        if !valid {
            diagnostics.push(error(
                code,
                &format!(
                    "Managed host file is missing or conflicts: {}",
                    path.display()
                ),
                "Run privileged setup again.",
            ));
        }
    }

    fn inspect_pf_conf(&self, diagnostics: &mut Vec<Diagnostic>) {
        let valid = matches!(
            self.files.metadata(Path::new("/etc/pf.conf")),
            Ok(Some(HostFileMetadata {
                kind: HostFileKind::Regular,
                owner_id: 0,
                ..
            }))
        ) && self
            .files
            .read(Path::new("/etc/pf.conf"), MAX_FILE_BYTES)
            .is_ok_and(|bytes| {
                bytes
                    .windows(PF_CONF_BLOCK.len())
                    .any(|window| window == PF_CONF_BLOCK.as_bytes())
            });
        if !valid {
            diagnostics.push(error(
                "pfConfigurationInvalid",
                "The PF configuration does not contain the owned anchor block.",
                "Run privileged setup again.",
            ));
        }
    }

    fn inspect_pf(&self, diagnostics: &mut Vec<Diagnostic>) {
        let request = inspect_command("/sbin/pfctl", ["-a", "dev.owndomains", "-s", "nat"]);
        match self.run_bounded(&request) {
            Ok(output) if output.status == 0 && !output.stdout.is_empty() => {}
            Ok(output)
                if output.status != 0
                    && String::from_utf8_lossy(&output.stderr)
                        .to_ascii_lowercase()
                        .contains("permission denied") =>
            {
                diagnostics.push(warning(
                    "pfAnchorUnverifiable",
                    "The Own Domains PF anchor cannot be inspected without administrator privileges.",
                    "The privileged setup verified the anchor; check PF manually if routing fails.",
                ));
            }
            _ => diagnostics.push(error(
                "pfAnchorInactive",
                "The Own Domains PF anchor is not active.",
                "Run privileged setup again.",
            )),
        }
    }

    fn inspect_trust(&self, diagnostics: &mut Vec<Diagnostic>) {
        let certificate = self.paths.ca_certificate.to_string_lossy();
        let request = inspect_command(
            "/usr/bin/security",
            ["verify-cert", "-c", certificate.as_ref(), "-p", "ssl"],
        );
        if !matches!(self.run_bounded(&request), Ok(output) if output.status == 0) {
            diagnostics.push(error(
                "caNotTrusted",
                "The Own Domains root certificate is not trusted.",
                "Run privileged setup again.",
            ));
        }
    }

    fn inspect_sidecar<const N: usize>(
        &self,
        name: &'static str,
        path: &Path,
        spec: &SidecarSpec,
        args: [&str; N],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if !matches!(
            self.files.metadata(path),
            Ok(Some(HostFileMetadata {
                kind: HostFileKind::Regular,
                ..
            }))
        ) {
            diagnostics.push(error(
                "sidecarMissing",
                &format!("{name} is missing from application resources."),
                "Reinstall the application.",
            ));
            return;
        }
        if self.files.sha256(path, 128 * 1024 * 1024).ok().as_deref() != Some(spec.sha256.as_str())
        {
            diagnostics.push(error(
                "sidecarDigestMismatch",
                &format!("{name} does not match the signed build manifest."),
                "Reinstall the application.",
            ));
        }
        let request = inspect_command(path.to_string_lossy().as_ref(), args);
        if !matches!(self.run_bounded(&request), Ok(output) if output.status == 0 && String::from_utf8_lossy(&output.stdout).contains(&spec.version))
        {
            diagnostics.push(error(
                "sidecarVersionMismatch",
                &format!("{name} reports an unexpected version."),
                "Reinstall the application.",
            ));
        }
    }

    fn inspect_ports(&self, diagnostics: &mut Vec<Diagnostic>) {
        let request = inspect_command(
            "/usr/sbin/lsof",
            [
                "-nP",
                "-iTCP:53",
                "-iUDP:53",
                "-iTCP:80",
                "-iTCP:443",
                "-iTCP:5354",
                "-iUDP:5354",
                "-iTCP:8080",
                "-iTCP:8443",
                "-iTCP:4781",
            ],
        );
        match self.run_bounded(&request) {
            Ok(output) if output.status != 0 || output.stdout.is_empty() => {}
            Ok(output) => diagnostics.push(error(
                "portConflict",
                &format!(
                    "A required port is occupied: {}",
                    first_line(&output.stdout)
                ),
                "Stop the conflicting process and retry.",
            )),
            Err(message) => diagnostics.push(error(
                "inspectionFailed",
                &format!("Unable to inspect required ports: {message}"),
                "Verify macOS command-line tools and retry.",
            )),
        }
    }

    fn run_bounded(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        let output = self.commands.run(command)?;
        if output.stdout.len().saturating_add(output.stderr.len()) > MAX_COMMAND_BYTES {
            return Err("command output exceeded 64 KiB".into());
        }
        Ok(output)
    }
}

pub fn build_host_setup_plan(options: &MacOsSetupOptions) -> Result<HostSetupPlan, HostSetupError> {
    validate_options(options)?;
    let paths = RuntimePaths::new(
        options.state_directory.clone(),
        options.resource_directory.clone(),
        options.backup_directory.clone(),
        options.architecture,
    )?;
    let mut interfaces = options
        .covered_interfaces
        .iter()
        .map(|interface| interface.name.clone())
        .collect::<Vec<_>>();
    interfaces.sort();
    let anchor = render_anchor_for(&interfaces);
    let mut artifacts = vec![
        system_artifact("/etc/resolver/own", RESOLVER_CONTENT, false),
        system_artifact(ANCHOR_PATH, &anchor, false),
        system_artifact("/etc/pf.conf", PF_CONF_BLOCK, true),
    ];
    let manifest = HostInstallManifest {
        schema_version: 2,
        user_name: options.user_name.clone(),
        user_id: options.user_id,
        backup_directory: options.backup_directory.clone(),
        interfaces,
        artifacts: artifacts
            .iter()
            .map(|artifact| ManifestArtifact {
                path: artifact.path.clone(),
                sha256: sha256(artifact.content.as_bytes()),
                mode: artifact.mode,
                owner_id: artifact.owner_id,
                managed_block: artifact.managed_block,
            })
            .collect(),
        sidecars: options.sidecars.clone(),
    };
    artifacts.push(HostArtifact {
        path: paths.install_manifest.clone(),
        content: format!("{}\n", serde_json::to_string_pretty(&manifest)?),
        mode: 0o600,
        owner_id: options.user_id,
        backup_existing: false,
        managed_block: false,
    });
    let ca_certificate =
        paths
            .ca_certificate
            .to_str()
            .ok_or_else(|| HostSetupError::NonUtf8Path {
                label: "CA certificate path",
                path: paths.ca_certificate.clone(),
            })?;
    let commands = vec![
        command(
            "/usr/bin/security",
            [
                "add-trusted-cert",
                "-d",
                "-r",
                "trustRoot",
                "-k",
                "/Library/Keychains/System.keychain",
                ca_certificate,
            ],
        ),
        command("/sbin/pfctl", ["-nf", "/etc/pf.conf"]),
        command("/sbin/pfctl", ["-f", "/etc/pf.conf"]),
        command("/sbin/pfctl", ["-a", "dev.owndomains", "-f", ANCHOR_PATH]),
        command("/sbin/pfctl", ["-E"]),
    ];
    let plan = HostSetupPlan {
        paths,
        artifacts,
        commands,
        manifest,
    };
    validate_host_setup_plan(&plan)?;
    Ok(plan)
}

pub fn validate_host_setup_plan(plan: &HostSetupPlan) -> Result<(), HostSetupError> {
    let mut paths = HashSet::new();
    for artifact in &plan.artifacts {
        if !artifact.path.is_absolute() {
            return Err(HostSetupError::RelativeArtifact(artifact.path.clone()));
        }
        if !paths.insert(&artifact.path) {
            return Err(HostSetupError::DuplicateArtifact(artifact.path.clone()));
        }
    }
    Ok(())
}

fn validate_options(options: &MacOsSetupOptions) -> Result<(), HostSetupError> {
    if options.user_id == 0
        || options.user_name.is_empty()
        || !options
            .user_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(HostSetupError::InvalidUser(options.user_name.clone()));
    }
    if options.sidecars.schema_version != 1 {
        return Err(HostSetupError::InvalidManifestSchema(
            options.sidecars.schema_version,
        ));
    }
    if !is_eligible_interface(&options.interface) {
        return Err(HostSetupError::InvalidInterface(options.interface.clone()));
    }
    if options.covered_interfaces.is_empty() {
        return Err(HostSetupError::EmptyInterfaceCoverage);
    }
    let mut interface_names = HashSet::new();
    for interface in &options.covered_interfaces {
        if !is_eligible_interface(&interface.name) {
            return Err(HostSetupError::InvalidInterface(interface.name.clone()));
        }
        if !interface_names.insert(interface.name.as_str()) {
            return Err(HostSetupError::DuplicateInterface(interface.name.clone()));
        }
    }
    let selected = options
        .covered_interfaces
        .iter()
        .find(|interface| interface.name.as_str() == options.interface.as_str())
        .ok_or_else(|| HostSetupError::InterfaceNotCovered(options.interface.clone()))?;
    if options.lan_address.is_unspecified()
        || options.lan_address.is_loopback()
        || options.lan_address.is_multicast()
        || selected.primary_ipv4 != Some(options.lan_address)
    {
        return Err(HostSetupError::AddressMismatch {
            interface: options.interface.clone(),
            address: options.lan_address,
        });
    }
    for (name, sidecar) in [
        ("CoreDNS", &options.sidecars.coredns),
        ("Caddy", &options.sidecars.caddy),
    ] {
        if sidecar.version.is_empty()
            || !sidecar
                .version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(HostSetupError::InvalidVersion { sidecar: name });
        }
        if sidecar.sha256.len() != 64
            || !sidecar.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(HostSetupError::InvalidDigest { sidecar: name });
        }
    }
    for (label, path) in [
        ("home directory", &options.home_directory),
        ("state directory", &options.state_directory),
        ("resource directory", &options.resource_directory),
        ("backup directory", &options.backup_directory),
    ] {
        if !path.is_absolute() {
            return Err(HostSetupError::RelativePath {
                label,
                path: path.clone(),
            });
        }
    }
    if !options.state_directory.starts_with(&options.home_directory)
        || !options
            .backup_directory
            .starts_with(&options.state_directory)
    {
        return Err(HostSetupError::InvalidDirectoryLayout);
    }
    Ok(())
}

fn render_anchor_for(interfaces: &[String]) -> String {
    const SERVICES: [(&str, u16, u16); 4] = [
        ("udp", 53, 5354),
        ("tcp", 53, 5354),
        ("tcp", 80, 8080),
        ("tcp", 443, 8443),
    ];
    let mut output = String::new();
    for (protocol, source, target) in SERVICES {
        output.push_str(&format!(
            "rdr pass on lo0 inet proto {protocol} from any to 127.0.0.1 port {source} -> 127.0.0.1 port {target}\n"
        ));
    }
    for interface in interfaces {
        for (protocol, source, target) in SERVICES {
            // Redirect to the interface's current address instead of loopback.
            output.push_str(&format!(
                "rdr pass on lo0 inet proto {protocol} from any to ({interface}:0) port {source} -> ({interface}:0) port {target}\n"
            ));
        }
        for (protocol, source, target) in SERVICES {
            output.push_str(&format!(
                "rdr pass on {interface} inet proto {protocol} from any to ({interface}:0) port {source} -> ({interface}:0) port {target}\n"
            ));
        }
    }
    output
}

fn system_artifact(path: &str, content: &str, managed_block: bool) -> HostArtifact {
    HostArtifact {
        path: PathBuf::from(path),
        content: content.into(),
        mode: 0o644,
        owner_id: 0,
        backup_existing: true,
        managed_block,
    }
}

fn command<const N: usize>(program: &str, args: [&str; N]) -> HostCommand {
    HostCommand {
        program: PathBuf::from(program),
        args: args.into_iter().map(String::from).collect(),
        timeout: Duration::from_secs(10),
    }
}

fn inspect_command<const N: usize>(program: &str, args: [&str; N]) -> HostCommand {
    HostCommand {
        program: PathBuf::from(program),
        args: args.into_iter().map(String::from).collect(),
        timeout: Duration::from_secs(2),
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn error(code: &'static str, message: &str, remedy: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        status: DiagnosticStatus::Error,
        message: message.into(),
        remedy: Some(remedy.into()),
    }
}

fn warning(code: &'static str, message: &str, remedy: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        status: DiagnosticStatus::Warning,
        message: message.into(),
        remedy: Some(remedy.into()),
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("unknown listener")
        .chars()
        .take(240)
        .collect()
}

pub struct SystemHostFileSystem;

impl HostFileSystem for SystemHostFileSystem {
    fn metadata(&self, path: &Path) -> Result<Option<HostFileMetadata>, String> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("unable to inspect {}: {error}", path.display()));
            }
        };
        let file_type = metadata.file_type();
        let kind = if file_type.is_file() {
            HostFileKind::Regular
        } else if file_type.is_symlink() {
            HostFileKind::Symlink
        } else {
            HostFileKind::Other
        };
        #[cfg(unix)]
        let (owner_id, mode) = {
            use std::os::unix::fs::MetadataExt;
            (metadata.uid(), metadata.mode() & 0o7777)
        };
        #[cfg(not(unix))]
        let (owner_id, mode) = (0, 0);
        Ok(Some(HostFileMetadata {
            kind,
            owner_id,
            mode,
        }))
    }

    fn read(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
        let mut bytes = Vec::new();
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
        if bytes.len() > max_bytes {
            return Err(format!(
                "{} exceeds the {max_bytes}-byte read limit",
                path.display()
            ));
        }
        Ok(bytes)
    }

    fn sha256(&self, path: &Path, max_bytes: u64) -> Result<String, String> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|error| format!("unable to hash {}: {error}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 16 * 1024];
        let mut reader = file.take(max_bytes + 1);
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("unable to hash {}: {error}", path.display()))?;
            if read == 0 {
                break;
            }
            bytes += read as u64;
            if bytes > max_bytes {
                return Err(format!(
                    "{} exceeds the {max_bytes}-byte hash limit",
                    path.display()
                ));
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        use std::{
            process::{Command, Stdio},
            thread,
            time::Instant,
        };

        let mut child = Command::new(&command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("unable to run {}: {error}", command.program.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "unable to capture command stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "unable to capture command stderr".to_string())?;
        let stdout = thread::spawn(move || read_bounded_command_output(stdout));
        let stderr = thread::spawn(move || read_bounded_command_output(stderr));
        let deadline = Instant::now() + command.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} did not finish within {:?}",
                        command.program.display(),
                        command.timeout
                    ));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "unable to wait for {}: {error}",
                        command.program.display()
                    ));
                }
            }
        };
        let stdout = stdout
            .join()
            .map_err(|_| "command stdout reader panicked".to_string())??;
        let stderr = stderr
            .join()
            .map_err(|_| "command stderr reader panicked".to_string())??;
        Ok(HostCommandOutput {
            status: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }
}

fn read_bounded_command_output(mut reader: impl std::io::Read) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_COMMAND_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("unable to read command output: {error}"))?;
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(format!(
            "command output exceeds the {MAX_COMMAND_BYTES}-byte limit"
        ));
    }
    Ok(bytes)
}
