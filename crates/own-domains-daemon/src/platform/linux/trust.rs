//! Trust certificate fingerprinting for the Linux certificate install.
//!
//! Accepts PEM (exactly one block) or raw DER, and requires a DER SEQUENCE.
//! Pure parsing and hashing: no keychain, no `update-ca-certificates`.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use super::error::LinuxArtifactError;

const PEM_BEGIN: &[u8] = b"-----BEGIN CERTIFICATE-----";
const PEM_END: &[u8] = b"-----END CERTIFICATE-----";

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut total = 0;
    let mut rest = haystack;
    while let Some(at) = find(rest, needle) {
        total += 1;
        rest = &rest[at + needle.len()..];
    }
    total
}

fn require_sequence(der: &[u8]) -> Result<(), LinuxArtifactError> {
    match der.first() {
        Some(0x30) => Ok(()),
        _ => Err(LinuxArtifactError::InvalidCertificate(
            "certificate is not a DER SEQUENCE".into(),
        )),
    }
}

/// Decode PEM (exactly one block) or accept raw DER.
pub fn certificate_der(input: &[u8]) -> Result<Vec<u8>, LinuxArtifactError> {
    if find(input, PEM_BEGIN).is_none() {
        if input.is_empty() {
            return Err(LinuxArtifactError::InvalidCertificate(
                "certificate is empty".into(),
            ));
        }
        let der = input.to_vec();
        require_sequence(&der)?;
        return Ok(der);
    }
    if count(input, PEM_BEGIN) != 1 || count(input, PEM_END) != 1 {
        return Err(LinuxArtifactError::InvalidCertificate(
            "certificate must contain exactly one PEM block".into(),
        ));
    }
    let begin_at = find(input, PEM_BEGIN).unwrap_or(0);
    let end_rel = find(&input[begin_at + PEM_BEGIN.len()..], PEM_END);
    let Some(end_rel) = end_rel else {
        return Err(LinuxArtifactError::InvalidCertificate(
            "certificate block is missing its footer".into(),
        ));
    };
    let body_end = begin_at + PEM_BEGIN.len() + end_rel;
    let body: Vec<u8> = input[begin_at + PEM_BEGIN.len()..body_end]
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let der = STANDARD.decode(&body).map_err(|_| {
        LinuxArtifactError::InvalidCertificate("certificate body is not valid base64".into())
    })?;
    require_sequence(&der)?;
    Ok(der)
}

/// Lowercase hex SHA-256 over DER bytes.
pub fn certificate_sha256(der: &[u8]) -> String {
    format!("{:x}", Sha256::digest(der))
}

fn valid_fingerprint(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn verify_certificate_fingerprint(
    der: &[u8],
    expected: &str,
) -> Result<(), LinuxArtifactError> {
    if !valid_fingerprint(expected) {
        return Err(LinuxArtifactError::InvalidCertificate(
            "expected fingerprint must be 64 lowercase hex characters".into(),
        ));
    }
    let actual = certificate_sha256(der);
    if actual == expected {
        Ok(())
    } else {
        Err(LinuxArtifactError::CertificateFingerprintMismatch {
            expected: expected.to_string(),
            actual,
        })
    }
}
