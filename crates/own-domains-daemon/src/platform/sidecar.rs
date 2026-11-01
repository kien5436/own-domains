//! Pinned sidecar releases shared by every host adapter.
//!
//! macOS, Linux, and Windows verify the same CoreDNS and Caddy versions by
//! SHA-256; only the per-platform asset names differ.

use serde::{Deserialize, Serialize};

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
