//! Trust certificate fingerprinting against the pinned fixture.
//!
//! Pure parsing and hashing: no keychain, no `update-ca-certificates`.

use own_domains_daemon::platform::linux::trust::{
    certificate_der, certificate_sha256, verify_certificate_fingerprint,
};

const CERTIFICATE_PEM: &[u8] = include_bytes!("../../../tests/fixtures/sidecars/certificate.pem");
const EXPECTED_FINGERPRINT: &str =
    "15a63910f999a5e2e8493ccf7e57b0c7c85fccc70ee8eb62d8f309a4e013be6f";

#[test]
fn pem_and_der_inputs_share_one_fingerprint() {
    let from_pem = certificate_sha256(&certificate_der(CERTIFICATE_PEM).unwrap());
    assert_eq!(from_pem, EXPECTED_FINGERPRINT);
    assert_eq!(from_pem.len(), 64);
    assert!(
        from_pem
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );

    let raw = certificate_der(CERTIFICATE_PEM).unwrap();
    assert_eq!(certificate_sha256(&raw), from_pem);

    verify_certificate_fingerprint(&raw, EXPECTED_FINGERPRINT).unwrap();
    assert!(verify_certificate_fingerprint(&raw, &"0".repeat(64)).is_err());
}

#[test]
fn malformed_inputs_are_rejected() {
    assert!(certificate_der(b"").is_err());
    assert!(
        certificate_der(b"-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n").is_err()
    );
    assert!(certificate_der(b"not a certificate").is_err());
    assert!(certificate_der(b"\x04\x03\x02\x01").is_err());
    // Two blocks are ambiguous: refuse instead of picking one.
    let doubled = [CERTIFICATE_PEM, CERTIFICATE_PEM].concat();
    assert!(certificate_der(&doubled).is_err());
    // A missing footer is a missing block.
    let no_footer = CERTIFICATE_PEM
        .windows(b"-----END".len())
        .position(|window| window == b"-----END")
        .map(|at| CERTIFICATE_PEM[..at].to_vec())
        .unwrap();
    assert!(certificate_der(&no_footer).is_err());
    // Fingerprints must be 64 lowercase hex characters.
    let raw = certificate_der(CERTIFICATE_PEM).unwrap();
    assert!(verify_certificate_fingerprint(&raw, "ABC").is_err());
    assert!(verify_certificate_fingerprint(&raw, &"A".repeat(64)).is_err());
}
