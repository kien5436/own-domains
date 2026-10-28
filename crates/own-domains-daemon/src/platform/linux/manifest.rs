//! Linux install manifest: rendering, parsing, and validation.
//!
//! The manifest pins every host artifact a setup writes: the fixed paths,
//! their SHA-256 digests, the covered interfaces, the sidecar pins, and the
//! trust fingerprint. Byte-exact rendering; strict validation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    error::LinuxArtifactError,
    interfaces::{MAX_INTERFACES, is_eligible_interface_name},
    nftables::NFTABLES_TABLE,
    systemd::{NFTABLES_UNIT_NAME, RESOLVER_UNIT_NAME},
};
use crate::platform::SidecarManifest;

pub const BACKUP_DIRECTORY: &str = "/etc/own-domains/backups";
pub const RESOLVER_LINK: &str = "own-domains0";

const EXPECTED_ARTIFACTS: &[(&str, LinuxArtifactKind)] = &[
    (
        "/etc/own-domains/nftables.conf",
        LinuxArtifactKind::Nftables,
    ),
    (
        "/etc/systemd/system/own-domains-nftables.service",
        LinuxArtifactKind::SystemdUnit,
    ),
    (
        "/etc/systemd/system/own-domains-resolver.service",
        LinuxArtifactKind::SystemdUnit,
    ),
    (
        "/usr/local/share/ca-certificates/own-domains.crt",
        LinuxArtifactKind::TrustCertificate,
    ),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinuxArtifactKind {
    Nftables,
    SystemdUnit,
    TrustCertificate,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxManifestArtifact {
    pub path: String,
    pub sha256: String,
    pub mode: u32,
    pub owner_id: u32,
    pub group_id: u32,
    pub kind: LinuxArtifactKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxInstallManifest {
    pub schema_version: u32,
    pub platform: String,
    pub host_integration_version: u32,
    pub user_name: String,
    pub user_id: u32,
    pub backup_directory: String,
    pub interfaces: Vec<String>,
    pub artifacts: Vec<LinuxManifestArtifact>,
    pub sidecars: SidecarManifest,
    pub ca_fingerprint_sha256: String,
    pub nftables_table: String,
    pub resolver_link: String,
    pub systemd_units: Vec<String>,
}

pub struct LinuxArtifactContent<'a> {
    pub path: &'a str,
    pub kind: LinuxArtifactKind,
    pub content: &'a [u8],
}

fn hex_sha256(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn is_lower_hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> LinuxArtifactError {
    LinuxArtifactError::InvalidManifest(message.to_string())
}

fn valid_user_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[allow(clippy::too_many_arguments)]
pub fn build_linux_manifest(
    user_name: &str,
    user_id: u32,
    interfaces: &[impl AsRef<str>],
    sidecars: SidecarManifest,
    ca_fingerprint_sha256: &str,
    artifacts: &[LinuxArtifactContent<'_>],
) -> LinuxInstallManifest {
    let mut sorted: Vec<String> = interfaces
        .iter()
        .map(|name| name.as_ref().to_string())
        .collect();
    sorted.sort();
    LinuxInstallManifest {
        schema_version: 1,
        platform: "linux".to_string(),
        host_integration_version: 1,
        user_name: user_name.to_string(),
        user_id,
        backup_directory: BACKUP_DIRECTORY.to_string(),
        interfaces: sorted,
        artifacts: artifacts
            .iter()
            .map(|artifact| LinuxManifestArtifact {
                path: artifact.path.to_string(),
                sha256: hex_sha256(artifact.content),
                mode: 0o644,
                owner_id: 0,
                group_id: 0,
                kind: artifact.kind,
            })
            .collect(),
        sidecars,
        ca_fingerprint_sha256: ca_fingerprint_sha256.to_string(),
        nftables_table: NFTABLES_TABLE.to_string(),
        resolver_link: RESOLVER_LINK.to_string(),
        systemd_units: vec![
            NFTABLES_UNIT_NAME.to_string(),
            RESOLVER_UNIT_NAME.to_string(),
        ],
    }
}

pub fn render_linux_manifest(
    manifest: &LinuxInstallManifest,
) -> Result<String, LinuxArtifactError> {
    serde_json::to_string_pretty(manifest)
        .map(|rendered| format!("{rendered}\n"))
        .map_err(|error| invalid(&format!("unable to render the manifest: {error}")))
}

pub fn parse_linux_manifest(bytes: &[u8]) -> Result<LinuxInstallManifest, LinuxArtifactError> {
    let manifest: LinuxInstallManifest = serde_json::from_slice(bytes)
        .map_err(|error| invalid(&format!("unable to parse the manifest: {error}")))?;
    validate_linux_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_linux_manifest(manifest: &LinuxInstallManifest) -> Result<(), LinuxArtifactError> {
    if manifest.schema_version != 1 {
        return Err(invalid("schemaVersion must be 1"));
    }
    if manifest.platform != "linux" {
        return Err(invalid("platform must be linux"));
    }
    if manifest.host_integration_version != 1 {
        return Err(invalid("hostIntegrationVersion must be 1"));
    }
    if !valid_user_name(&manifest.user_name) {
        return Err(invalid("invalid user name"));
    }
    if manifest.user_id == 0 {
        return Err(LinuxArtifactError::InvalidUserId);
    }
    if manifest.backup_directory != BACKUP_DIRECTORY {
        return Err(invalid("unexpected backup directory"));
    }
    validate_manifest_interfaces(&manifest.interfaces)?;
    validate_manifest_artifacts(&manifest.artifacts)?;
    validate_manifest_sidecars(&manifest.sidecars)?;
    if !is_lower_hex64(&manifest.ca_fingerprint_sha256) {
        return Err(invalid("invalid CA fingerprint"));
    }
    if manifest.nftables_table != NFTABLES_TABLE {
        return Err(invalid("unexpected nftables table"));
    }
    if manifest.resolver_link != RESOLVER_LINK {
        return Err(invalid("unexpected resolver link"));
    }
    if manifest.systemd_units
        != [
            NFTABLES_UNIT_NAME.to_string(),
            RESOLVER_UNIT_NAME.to_string(),
        ]
    {
        return Err(invalid("unexpected systemd units"));
    }
    Ok(())
}

fn validate_manifest_interfaces(interfaces: &[String]) -> Result<(), LinuxArtifactError> {
    if interfaces.is_empty() {
        return Err(LinuxArtifactError::EmptyInterfaces);
    }
    if interfaces.len() > MAX_INTERFACES {
        return Err(LinuxArtifactError::TooManyInterfaces(interfaces.len()));
    }
    for name in interfaces {
        if !is_eligible_interface_name(name) {
            return Err(LinuxArtifactError::InvalidInterface(name.clone()));
        }
    }
    for pair in interfaces.windows(2) {
        if pair[0] == pair[1] {
            return Err(LinuxArtifactError::DuplicateInterface(pair[0].clone()));
        }
        if pair[0] > pair[1] {
            return Err(invalid("interfaces must be sorted"));
        }
    }
    Ok(())
}

fn validate_manifest_artifacts(
    artifacts: &[LinuxManifestArtifact],
) -> Result<(), LinuxArtifactError> {
    if artifacts.len() != EXPECTED_ARTIFACTS.len() {
        return Err(invalid("unexpected artifact set"));
    }
    for (artifact, (path, kind)) in artifacts.iter().zip(EXPECTED_ARTIFACTS.iter()) {
        if artifact.path != *path || artifact.kind != *kind {
            return Err(invalid(&format!("unexpected artifact: {}", artifact.path)));
        }
        if !is_lower_hex64(&artifact.sha256) {
            return Err(invalid(&format!("invalid digest for {}", artifact.path)));
        }
        if artifact.mode != 0o644 {
            return Err(invalid(&format!("unexpected mode for {}", artifact.path)));
        }
        if artifact.owner_id != 0 || artifact.group_id != 0 {
            return Err(invalid(&format!(
                "unexpected ownership for {}",
                artifact.path
            )));
        }
    }
    Ok(())
}

fn validate_manifest_sidecars(sidecars: &SidecarManifest) -> Result<(), LinuxArtifactError> {
    if sidecars.schema_version != 1 {
        return Err(invalid("sidecar schemaVersion must be 1"));
    }
    for (name, spec) in [("coredns", &sidecars.coredns), ("caddy", &sidecars.caddy)] {
        if spec.version.is_empty() {
            return Err(invalid(&format!("sidecar {name} version is required")));
        }
        if !is_lower_hex64(&spec.sha256) {
            return Err(invalid(&format!("sidecar {name} digest is invalid")));
        }
    }
    Ok(())
}
