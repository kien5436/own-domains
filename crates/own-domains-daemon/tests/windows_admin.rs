#![cfg(target_os = "windows")]

//! Caddy admin over AF_UNIX on Windows, against an in-test stub server.
//!
//! The stub speaks just enough HTTP/1.1 to prove framing, status handling,
//! and path routing; interop with real Caddy stays a VM acceptance step.

use own_domains_daemon::probe::CaddyAdminClient;
use std::{path::PathBuf, sync::Arc};

fn socket_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("od-admin-{tag}-{}.sock", std::process::id()))
}

fn startup_wsa() {
    use windows::Win32::Networking::WinSock::{WSADATA, WSAStartup};
    unsafe {
        let mut data = std::mem::zeroed::<WSADATA>();
        assert_eq!(WSAStartup(0x202, &mut data), 0);
    }
}

/// `sockaddr_un` as the OS lays it out; the `windows` crate does not ship it.
#[repr(C)]
struct SockAddrUn {
    sun_family: u16,
    sun_path: [u8; 108],
}

const AF_UNIX_U16: u16 = 1;

fn send_all(handle: usize, buf: &[u8]) {
    use windows::Win32::Networking::WinSock::{SEND_RECV_FLAGS, SOCKET, send};
    let mut sent = 0;
    while sent < buf.len() {
        let done = unsafe { send(SOCKET(handle), &buf[sent..], SEND_RECV_FLAGS(0)) };
        assert!(done > 0, "stub send failed");
        sent += done as usize;
    }
}

fn recv_until(handle: usize, marker: &[u8]) -> Vec<u8> {
    use windows::Win32::Networking::WinSock::{SEND_RECV_FLAGS, SOCKET, recv};
    let mut out = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let done = unsafe { recv(SOCKET(handle), &mut chunk, SEND_RECV_FLAGS(0)) };
        assert!(done > 0, "stub recv failed");
        out.extend_from_slice(&chunk[..done as usize]);
        if out.windows(marker.len()).any(|w| w == marker) {
            return out;
        }
    }
}

/// Serve exactly `connections` replies, recording `METHOD path` per connection.
fn spawn_stub(
    socket_path: PathBuf,
    respond_with: &'static str,
    seen: Arc<std::sync::Mutex<Vec<String>>>,
    connections: usize,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        startup_wsa();
        use windows::Win32::Networking::WinSock::{
            SOCK_STREAM, SOCKADDR, SOCKET, accept, bind, listen, socket,
        };
        let _ = std::fs::remove_file(&socket_path);
        let listener = unsafe { socket(AF_UNIX_U16 as i32, SOCK_STREAM, 0).unwrap() };
        let mut addr = SockAddrUn {
            sun_family: AF_UNIX_U16,
            sun_path: [0; 108],
        };
        let bytes = socket_path.as_os_str().as_encoded_bytes();
        assert!(
            bytes.len() + 1 < addr.sun_path.len(),
            "socket path too long"
        );
        addr.sun_path[..bytes.len()].copy_from_slice(bytes);
        let addr_len = (2 + bytes.len() + 1) as i32;
        unsafe {
            assert_eq!(
                bind(
                    listener,
                    &addr as *const SockAddrUn as *const SOCKADDR,
                    addr_len,
                ),
                0
            );
            assert_eq!(listen(listener, connections as i32), 0);
        }
        for _ in 0..connections {
            let conn = unsafe { accept(listener, None, None).unwrap() };
            let handle = conn.0;
            let head = recv_until(handle, b"\r\n\r\n");
            let text = String::from_utf8_lossy(&head);
            let mut lines = text.lines();
            let request_line = lines.next().unwrap_or("").to_string();
            let mut length = 0usize;
            for line in lines {
                if line.is_empty() {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = Vec::new();
            while body.len() < length {
                let mut one = [0u8; 1024];
                let done = unsafe {
                    windows::Win32::Networking::WinSock::recv(
                        SOCKET(handle),
                        &mut one,
                        windows::Win32::Networking::WinSock::SEND_RECV_FLAGS(0),
                    )
                };
                assert!(done > 0);
                body.extend_from_slice(&one[..done as usize]);
            }
            seen.lock().unwrap().push(request_line);
            let reply = format!(
                "HTTP/1.1 {respond_with}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            );
            send_all(handle, reply.as_bytes());
            unsafe {
                assert_eq!(windows::Win32::Networking::WinSock::closesocket(conn), 0);
            }
        }
        unsafe {
            assert_eq!(
                windows::Win32::Networking::WinSock::closesocket(listener),
                0
            );
        }
        let _ = std::fs::remove_file(&socket_path);
    })
}

#[test]
fn admin_load_switches_config_and_is_ready_passes() {
    let path = socket_path("load");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runner = spawn_stub(path.clone(), "200 OK", seen.clone(), 2);
    let client = CaddyAdminClient::new(&path).unwrap();
    client
        .load(&serde_json::json!({"apps": {"http": {}}}))
        .unwrap();
    client.is_ready().unwrap();
    runner.join().unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].starts_with("POST /load "), "{}", seen[0]);
    assert!(seen[1].starts_with("GET /config/ "), "{}", seen[1]);
}

#[test]
fn admin_reports_failure_status() {
    let path = socket_path("fail");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runner = spawn_stub(path.clone(), "500 Internal Error", seen, 1);
    let client = CaddyAdminClient::new(&path).unwrap();
    let error = client
        .load(&serde_json::json!({"apps": {"http": {}}}))
        .unwrap_err();
    runner.join().unwrap();
    assert!(error.contains("500"), "{error}");
}

#[test]
fn admin_rejects_relative_socket() {
    assert!(CaddyAdminClient::new(std::path::Path::new("relative.sock")).is_err());
}
