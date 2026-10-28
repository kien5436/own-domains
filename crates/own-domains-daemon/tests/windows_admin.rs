#![cfg(target_os = "windows")]

//! Caddy admin over loopback TCP on Windows, against an in-test stub server.
//!
//! The stub speaks just enough HTTP/1.1 to prove framing, status handling,
//! and path routing; interop with real Caddy stays a VM acceptance step.

use own_domains_daemon::probe::CaddyAdminClient;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener},
    sync::Arc,
};

/// Serve exactly `connections` replies, recording `METHOD path` per connection.
fn spawn_stub(
    respond_with: &'static str,
    seen: Arc<std::sync::Mutex<Vec<String>>>,
    connections: usize,
) -> (SocketAddr, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let runner = std::thread::spawn(move || {
        for _ in 0..connections {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 1024];
            let head_end = loop {
                let done = stream.read(&mut chunk).unwrap();
                assert!(done > 0, "stub recv failed");
                raw.extend_from_slice(&chunk[..done]);
                if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
            let mut lines = head.lines();
            let request_line = lines.next().unwrap_or("").to_string();
            let mut length = 0usize;
            let mut host = String::new();
            for line in lines {
                if line.is_empty() {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if let Some(value) = line.strip_prefix("Host:") {
                    host = value.trim().to_string();
                }
            }
            while raw.len() - head_end < length {
                let done = stream.read(&mut chunk).unwrap();
                assert!(done > 0, "stub body recv failed");
                raw.extend_from_slice(&chunk[..done]);
            }
            seen.lock()
                .unwrap()
                .push(format!("{request_line} host={host}"));
            let reply = format!(
                "HTTP/1.1 {respond_with}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    (address, runner)
}

#[test]
fn admin_load_switches_config_and_is_ready_passes() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (address, runner) = spawn_stub("200 OK", seen.clone(), 2);
    let client = CaddyAdminClient::new_tcp(address).unwrap();
    client
        .load(&serde_json::json!({"apps": {"http": {}}}))
        .unwrap();
    client.is_ready().unwrap();
    runner.join().unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].starts_with("POST /load "), "{}", seen[0]);
    assert!(seen[1].starts_with("GET /config/ "), "{}", seen[1]);
    for entry in seen.iter() {
        assert!(
            entry.ends_with(&format!(" host={address}")),
            "{entry} (admin Host must carry the loopback port)"
        );
    }
}

#[test]
fn admin_reports_failure_status() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (address, runner) = spawn_stub("500 Internal Error", seen, 1);
    let client = CaddyAdminClient::new_tcp(address).unwrap();
    let error = client
        .load(&serde_json::json!({"apps": {"http": {}}}))
        .unwrap_err();
    runner.join().unwrap();
    assert!(error.contains("500"), "{error}");
}

#[test]
fn admin_rejects_non_loopback_endpoint() {
    assert!(CaddyAdminClient::new_tcp(SocketAddr::from(([10, 0, 0, 1], 2019))).is_err());
    assert!(CaddyAdminClient::new_tcp(SocketAddr::from(([0, 0, 0, 0], 2019))).is_err());
    assert!(CaddyAdminClient::new_tcp(SocketAddr::from((Ipv4Addr::LOCALHOST, 2019))).is_ok());
}
