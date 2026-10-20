use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainMapping {
    pub id: String,
    pub hostname: String,
    pub upstream_port: u16,
    pub tls: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub schema_version: u32,
    pub revision: u64,
    pub lan_address: String,
    pub upstream_dns: Vec<String>,
    pub mappings: Vec<DomainMapping>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            lan_address: String::new(),
            upstream_dns: vec!["1.1.1.1".into(), "9.9.9.9".into()],
            mappings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappingInput {
    #[serde(default)]
    pub id: Option<String>,
    pub hostname: String,
    pub upstream_port: u16,
    pub tls: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappingPatch {
    pub hostname: Option<String>,
    pub upstream_port: Option<u16>,
    pub tls: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyStatus {
    Applied,
    Pending,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub desired_revision: u64,
    pub applied_revision: u64,
    pub status: ApplyStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub status: DiagnosticStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
}
