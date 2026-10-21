use own_domains_daemon::SetupServer;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};

const CERTIFICATE: &str =
    "-----BEGIN CERTIFICATE-----\nY2VydGlmaWNhdGUtYnl0ZXM=\n-----END CERTIFICATE-----\n";

fn certificate_path() -> PathBuf {
    let path = std::env::temp_dir().join(format!("own-domains-setup-{}.pem", uuid::Uuid::new_v4()));
    fs::write(&path, CERTIFICATE).unwrap();
    path
}

fn request(server: &SetupServer, request: &str) -> String {
    let mut stream = TcpStream::connect(server.address()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

#[test]
fn serves_only_documented_setup_resources_with_security_headers() {
    let certificate = certificate_path();
    let server = SetupServer::bind(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        certificate.clone(),
    )
    .unwrap();

    let page = request(&server, "GET / HTTP/1.1\r\nHost: setup.own\r\n\r\n");
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(page.contains("Content-Type: text/html; charset=utf-8\r\n"));
    assert!(page.contains("X-Content-Type-Options: nosniff\r\n"));
    assert!(page.contains("Cache-Control: no-store\r\n"));
    assert!(body(&page).contains("Own Domains"));

    let pem = request(&server, "GET /ca.crt HTTP/1.1\r\nHost: setup.own\r\n\r\n");
    assert!(pem.contains("Content-Type: application/x-x509-ca-cert\r\n"));
    assert_eq!(body(&pem), CERTIFICATE);

    let profile = request(
        &server,
        "GET /ca.mobileconfig HTTP/1.1\r\nHost: setup.own\r\n\r\n",
    );
    assert!(profile.contains("Content-Type: application/x-apple-aspen-config\r\n"));
    assert!(body(&profile).contains("com.apple.security.root"));
    assert!(body(&profile).contains("Y2VydGlmaWNhdGUtYnl0ZXM="));

    let fingerprint = request(
        &server,
        "GET /fingerprint HTTP/1.1\r\nHost: setup.own\r\n\r\n",
    );
    let expected = Sha256::digest(b"certificate-bytes");
    assert_eq!(body(&fingerprint), format!("SHA256:{expected:X}\n"));

    let head = request(&server, "HEAD /ca.crt HTTP/1.1\r\nHost: setup.own\r\n\r\n");
    assert!(head.contains(&format!("Content-Length: {}\r\n", CERTIFICATE.len())));
    assert!(body(&head).is_empty());

    server.shutdown().unwrap();
    fs::remove_file(certificate).unwrap();
}

#[test]
fn rejects_non_loopback_and_unsafe_requests_and_handles_missing_certificate() {
    let certificate = certificate_path();
    assert!(SetupServer::bind(SocketAddr::from(([0, 0, 0, 0], 0)), certificate.clone()).is_err());

    let server =
        SetupServer::bind(SocketAddr::from(([127, 0, 0, 1], 0)), certificate.clone()).unwrap();
    assert!(
        request(&server, "POST / HTTP/1.1\r\nHost: setup.own\r\n\r\n")
            .starts_with("HTTP/1.1 405 Method Not Allowed")
    );
    assert!(
        request(&server, "GET /missing HTTP/1.1\r\nHost: setup.own\r\n\r\n")
            .starts_with("HTTP/1.1 404 Not Found")
    );
    assert!(
        request(
            &server,
            &format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(9000))
        )
        .starts_with("HTTP/1.1 431 Request Header Fields Too Large")
    );
    fs::remove_file(&certificate).unwrap();
    assert!(
        request(&server, "GET /ca.crt HTTP/1.1\r\nHost: setup.own\r\n\r\n")
            .starts_with("HTTP/1.1 404 Not Found")
    );
    server.shutdown().unwrap();
}

#[test]
fn cancellation_does_not_wait_for_an_incomplete_request() {
    let certificate = certificate_path();
    let server =
        SetupServer::bind(SocketAddr::from(([127, 0, 0, 1], 0)), certificate.clone()).unwrap();
    let mut stream = TcpStream::connect(server.address()).unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: setup.own\r\n")
        .unwrap();

    let started = std::time::Instant::now();
    server.shutdown().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(stream);
    fs::remove_file(certificate).unwrap();
}
