#![cfg(target_os = "windows")]

//! End to end over real sidecars on the VM (Gate C acceptance).
//!
//! Staging (PowerShell, once):
//!   mkdir $env:USERPROFILE\sidecars
//!   Invoke-WebRequest https://github.com/coredns/coredns/releases/download/v1.14.7/coredns_1.14.7_windows_amd64.zip -OutFile $env:USERPROFILE\sidecars\coredns.zip
//!   # expect 6B718A5ED57D7034B1E4C3DF500121307830DA291D15CF8387F41C297C6FD56
//!   Invoke-WebRequest https://github.com/caddyserver/caddy/releases/download/v2.11.7/caddy_2.11_7_windows_arm64.zip -OutFile $env:USERPROFILE\sidecars\caddy.zip
//!   # expect 5E232F5FD68968C288034D98F8B4A6B16F246DA6D5F1D3A8C7543B83327F1DE0BF,
//!   #   5D01D627768FBFD1DD254D7D53EB94589A40B126C29DEF10AC01365A3FE982
//!   Expand-Archive $env:USERPROFILE\sidecars\cored 기관Ncoredns.zip, $env:USERPROFILE\sidecars
//!   Expand-Archive $env:USERPROFILE\sidecars\caddy.zip, $env:USERPROFILE\sidecars
//!   Unblock-File $env:USERPROFILE\sidecars\*.exe
//!
//! Run with:
//!   $env:OWN_DOMAINS_TEST_COREDNS_EXE = "$env:USERPROFILE\sidecars\coredns.exe"
//!   $env:OWN_DOMAINS_TEST_CADDY_EXE = "$env:USERPROFILE\sidecars\caddy.exe"
//!   $env:OWN_DOMAINS_TEST_LAN = "<guest IPv4, e.g. 192.168.128.2>"
//!   cargo test -p own-domains-daemon --test windows_backend

use own_domains_core::{DomainMapping, Settings};
use own_domains_daemon::platform::windows::backend::{
    BackendReadiness, SystemProcessHost, SystemReadiness, WindowsAdapters, WindowsPaths,
    WindowsRuntimeBackend,
};

use own_domains_daemon::{RuntimeBackend, probe::CaddyAdminClient};
use own_domains_protocol::RuntimeStatus;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

fn staging(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| panic!("set {name} to the staged sidecar executable (see top of file)"))
}

fn settings(lan: Ipv4Addr, upstream_port: u16) -> Settings {
    Settings {
        lan_address: lan.to_string(),
        upstream_dns: vec!["1.1.1.1".into()],
        revision: 1,
        mappings: vec![DomainMapping {
            id: "app".into(),
            hostname: "app.own".into(),
            upstream_port,
            tls: false,
            enabled: true,
        }],
        ..Settings::default()
    }
}

fn read_response(mut stream: TcpStream, host: &str) -> String {
    let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut body = Vec::new();
    stream.read_to_end(&mut body).unwrap();
    String::from_utf8_lossy(&body).into_owned()
}

fn assert_port_closed(port: u16) {
    assert!(
        TcpStream::connect_timeout(
            &std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            Duration::from_secs(2),
        )
        .is_err(),
        "expected port {port} to be closed after stop"
    );
}

#[test]
fn backend_serves_dns_and_http_with_real_sidecars() {
    let coredns = staging("OWN_DOMAINS_TEST_COREDNS_EXE");
    let caddy = staging("OWN_DOMAINS_TEST_CADDY_EXE");
    let lan = std::env::var("OWN_DOMAINS_TEST_LAN")
        .map(|lan| {
            Ipv4Addr::from_str(lan.trim().trim_matches('"'))
                .unwrap_or_else(|_| panic!("OWN_DOMAINS_TEST_LAN must be plain IPv4, got {lan:?}"))
        })
        .expect("set OWN_DOMAINS_TEST_LAN to the guest IPv4 (see top of file)");

    for _ in 0..2 {
        let directory = std::env::temp_dir().join(format!("od-win-e2e-{}", uuid::Uuid::new_v4()));
        let runtime = directory.join("runtime");

        // Upstream responder for the mapped app.own name. Readiness polls and
        // the final assertion each take proxied connections, so serve until
        // the iteration signals done.
        let upstream = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        upstream.set_nonblocking(true).unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done_flag = done.clone();
        let responder = std::thread::spawn(move || {
            while !done_flag.load(std::sync::atomic::Ordering::Relaxed) {
                let (mut stream, _) = match upstream.accept() {
                    Ok(next) => next,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut head = [0u8; 1024];
                if stream.read(&mut head).is_err() {
                    continue;
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });

        let settings = settings(lan, upstream_port);
        let admin = CaddyAdminClient::new(runtime.join("caddy-admin.sock")).unwrap();
        let backend = WindowsRuntimeBackend::with_adapters(
            WindowsPaths {
                runtime_directory: runtime.clone(),
                coredns_exe: coredns.clone(),
                caddy_exe: caddy.clone(),
                admin_socket: runtime.join("caddy-admin.sock"),
            },
            WindowsAdapters {
                process_host: &SystemProcessHost,
                readiness: &SystemReadiness,
                caddy: &admin,
            },
        );

        let applied = backend.apply(&settings).unwrap();
        assert_eq!(applied.applied_revision, 1);
        assert!(matches!(backend.inspect().unwrap(), RuntimeStatus::Running));

        // DNS serves the LAN address for .own names.
        own_domains_daemon::platform::windows::backend::SystemReadiness
            .wait_dns("127.0.0.1", 53, &lan.to_string())
            .unwrap();

        // HTTP serves the mapped app through Caddy.
        let stream = TcpStream::connect_timeout(
            &std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 80)),
            Duration::from_secs(10),
        )
        .unwrap();
        let body = read_response(stream, "app.own");
        assert!(body.contains(" 200 "), "{body}");
        assert!(body.ends_with("ok"), "{body}");

        backend.stop().unwrap();
        assert!(matches!(backend.inspect().unwrap(), RuntimeStatus::Stopped));
        for port in [53, 80, 443] {
            assert_port_closed(port);
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        responder.join().unwrap();
    }
}
