use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LinuxArtifactError {
    #[error("no covered interfaces were provided")]
    EmptyInterfaces,
    #[error("too many covered interfaces: {0}")]
    TooManyInterfaces(usize),
    #[error("ineligible interface name: {0}")]
    InvalidInterface(String),
    #[error("duplicate interface name: {0}")]
    DuplicateInterface(String),
    #[error("invalid user id")]
    InvalidUserId,
    #[error("unable to parse `ip` output: {0}")]
    InvalidIpOutput(String),
    #[error("invalid sidecar manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid certificate: {0}")]
    InvalidCertificate(String),
    #[error("certificate fingerprint mismatch: expected {expected}, found {actual}")]
    CertificateFingerprintMismatch { expected: String, actual: String },
}
