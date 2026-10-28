//! Transport- and platform-neutral runtime probes.
//!
//! DNS/HTTP readiness checks and the Caddy admin client work over any
//! byte stream, so macOS, Linux, and Windows share them. Only the Caddy
//! admin endpoint differs: a Unix socket on macOS/Linux, loopback-only TCP
//! on Windows (Caddy on Windows cannot serve admin over AF_UNIX).

use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    path::PathBuf,
    time::Duration,
};

const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;

pub trait ReadinessProbe: Send + Sync {
    fn component(&self) -> &'static str;
    fn check(&self) -> Result<(), String>;
}

pub struct DnsReadinessProbe {
    address: Ipv4Addr,
    port: u16,
    expected: Ipv4Addr,
    timeout: Duration,
}

impl DnsReadinessProbe {
    pub fn new(address: Ipv4Addr, port: u16, expected: Ipv4Addr, timeout: Duration) -> Self {
        Self {
            address,
            port,
            expected,
            timeout,
        }
    }

    fn query(&self) -> Vec<u8> {
        let mut query = Vec::with_capacity(32);
        query.extend_from_slice(&0x4f44_u16.to_be_bytes());
        query.extend_from_slice(&0x0100_u16.to_be_bytes());
        query.extend_from_slice(&1_u16.to_be_bytes());
        query.extend_from_slice(&0_u16.to_be_bytes());
        query.extend_from_slice(&0_u16.to_be_bytes());
        query.extend_from_slice(&0_u16.to_be_bytes());
        for label in ["setup", "own"] {
            query.push(label.len() as u8);
            query.extend_from_slice(label.as_bytes());
        }
        query.push(0);
        query.extend_from_slice(&1_u16.to_be_bytes());
        query.extend_from_slice(&1_u16.to_be_bytes());
        query
    }
}

impl ReadinessProbe for DnsReadinessProbe {
    fn component(&self) -> &'static str {
        "CoreDNS"
    }

    fn check(&self) -> Result<(), String> {
        let query = self.query();
        self.check_udp(&query)?;
        self.check_tcp(&query)
    }
}

impl DnsReadinessProbe {
    fn check_udp(&self, query: &[u8]) -> Result<(), String> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|error| format!("unable to bind UDP probe socket: {error}"))?;
        socket
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the UDP probe timeout: {error}"))?;
        socket
            .send_to(query, (self.address, self.port))
            .map_err(|error| format!("DNS UDP request failed: {error}"))?;
        let mut response = [0_u8; 4096];
        let (length, _) = socket
            .recv_from(&mut response)
            .map_err(|error| format!("DNS UDP response failed: {error}"))?;
        check_dns_answer(&response[..length], self.expected)
    }

    fn check_tcp(&self, query: &[u8]) -> Result<(), String> {
        let socket = SocketAddr::from((self.address, self.port));
        let mut stream = TcpStream::connect_timeout(&socket, self.timeout)
            .map_err(|error| format!("DNS TCP connection failed: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the DNS TCP timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the DNS TCP timeout: {error}"))?;
        stream
            .write_all(&(query.len() as u16).to_be_bytes())
            .map_err(|error| format!("DNS TCP request failed: {error}"))?;
        stream
            .write_all(query)
            .map_err(|error| format!("DNS TCP request failed: {error}"))?;
        let mut length = [0_u8; 2];
        stream
            .read_exact(&mut length)
            .map_err(|error| format!("DNS TCP response failed: {error}"))?;
        let mut response = vec![0_u8; u16::from_be_bytes(length) as usize];
        stream
            .read_exact(&mut response)
            .map_err(|error| format!("DNS TCP response failed: {error}"))?;
        check_dns_answer(&response, self.expected)
    }
}

pub struct HttpReadinessProbe {
    address: Ipv4Addr,
    port: u16,
    host: String,
    path: String,
    expected_status: u16,
    timeout: Duration,
}

impl HttpReadinessProbe {
    pub fn new(
        address: Ipv4Addr,
        port: u16,
        host: impl Into<String>,
        path: impl Into<String>,
        expected_status: u16,
        timeout: Duration,
    ) -> Self {
        Self {
            address,
            port,
            host: host.into(),
            path: path.into(),
            expected_status,
            timeout,
        }
    }
}

impl ReadinessProbe for HttpReadinessProbe {
    fn component(&self) -> &'static str {
        "HTTP"
    }

    fn check(&self) -> Result<(), String> {
        let socket = SocketAddr::from((self.address, self.port));
        let mut stream = TcpStream::connect_timeout(&socket, self.timeout)
            .map_err(|error| format!("HTTP connection failed: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the HTTP timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the HTTP timeout: {error}"))?;
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.path, self.host
        )
        .map_err(|error| format!("HTTP request failed: {error}"))?;
        let (status, _) = read_http_response(&mut stream, MAX_HTTP_BODY_BYTES)?;
        if status != self.expected_status {
            return Err(format!(
                "HTTP response status {status}, expected {}",
                self.expected_status
            ));
        }
        Ok(())
    }
}

pub trait CaddyAdmin: Send + Sync {
    fn load(&self, config: &Value) -> Result<(), String>;
    fn is_ready(&self) -> Result<(), String>;
}

pub struct CaddyAdminClient {
    endpoint: AdminEndpoint,
    timeout: Duration,
    max_response_bytes: usize,
}

/// Unix sockets serve macOS/Linux; Windows uses loopback TCP (Caddy on
/// Windows cannot serve admin over AF_UNIX). Each side of the split is dead
/// code on the other platform by construction.
#[allow(dead_code)]
enum AdminEndpoint {
    /// macOS/Linux: permission-restricted Unix socket in the state dir.
    Unix(PathBuf),
    /// Windows: loopback-only TCP (Caddy on Windows cannot serve its admin
    /// API over AF_UNIX).
    Tcp(SocketAddr),
}

impl CaddyAdminClient {
    pub fn new(socket: impl Into<PathBuf>) -> Result<Self, String> {
        let socket = socket.into();
        if !socket.is_absolute() {
            return Err("Caddy admin socket must be absolute".into());
        }
        Ok(Self {
            endpoint: AdminEndpoint::Unix(socket),
            timeout: Duration::from_secs(2),
            max_response_bytes: MAX_HTTP_BODY_BYTES,
        })
    }

    pub fn new_tcp(address: SocketAddr) -> Result<Self, String> {
        if !address.ip().is_loopback() {
            return Err("Caddy admin TCP endpoint must be loopback-only".into());
        }
        Ok(Self {
            endpoint: AdminEndpoint::Tcp(address),
            timeout: Duration::from_secs(2),
            max_response_bytes: MAX_HTTP_BODY_BYTES,
        })
    }

    pub fn load(&self, config: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(config)
            .map_err(|error| format!("unable to serialize Caddy configuration: {error}"))?;
        self.request("POST", "/load", Some(&body), Some("application/json"))
            .map(|_| ())
    }

    pub fn is_ready(&self) -> Result<(), String> {
        self.request("GET", "/config/", None, None).map(|_| ())
    }

    #[cfg(unix)]
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        use std::os::unix::net::UnixStream;

        let AdminEndpoint::Unix(socket) = &self.endpoint else {
            return Err("Caddy admin TCP endpoint is unsupported on this platform".into());
        };
        let mut stream = UnixStream::connect(socket)
            .map_err(|error| format!("unable to connect to the Caddy admin socket: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the Caddy admin timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the Caddy admin timeout: {error}"))?;
        send_request(
            &mut stream,
            method,
            path,
            "localhost",
            body,
            content_type,
            self.max_response_bytes,
        )
    }

    #[cfg(windows)]
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let AdminEndpoint::Tcp(address) = &self.endpoint else {
            return Err("Caddy admin Unix socket is unsupported on Windows".into());
        };
        let mut stream = TcpStream::connect_timeout(address, self.timeout)
            .map_err(|error| format!("unable to connect to the Caddy admin endpoint: {error}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the Caddy admin timeout: {error}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| format!("unable to set the Caddy admin timeout: {error}"))?;
        send_request(
            &mut stream,
            method,
            path,
            &address.to_string(),
            body,
            content_type,
            self.max_response_bytes,
        )
    }

    #[cfg(not(any(unix, windows)))]
    fn request(
        &self,
        _method: &str,
        _path: &str,
        _body: Option<&[u8]>,
        _content_type: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        Err("Caddy admin Unix sockets are unsupported on this platform".into())
    }
}

/// One HTTP/1.1 exchange over an already-connected byte stream.
#[allow(clippy::too_many_arguments)]
fn send_request<S: Read + Write>(
    stream: &mut S,
    method: &str,
    path: &str,
    host: &str,
    body: Option<&[u8]>,
    content_type: Option<&str>,
    max_response_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(content_type) = content_type {
        request.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    if let Some(body) = body {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("Caddy admin request failed: {error}"))?;
    if let Some(body) = body {
        stream
            .write_all(body)
            .map_err(|error| format!("Caddy admin request failed: {error}"))?;
    }
    let (status, response) = read_http_response(&mut *stream, max_response_bytes)?;
    if !(200..300).contains(&status) {
        let detail = String::from_utf8_lossy(&response)
            .chars()
            .take(240)
            .collect::<String>();
        return Err(format!("Caddy admin returned status {status}: {detail}"));
    }
    Ok(response)
}

impl CaddyAdmin for CaddyAdminClient {
    fn load(&self, config: &Value) -> Result<(), String> {
        CaddyAdminClient::load(self, config)
    }

    fn is_ready(&self) -> Result<(), String> {
        CaddyAdminClient::is_ready(self)
    }
}

fn check_dns_answer(response: &[u8], expected: Ipv4Addr) -> Result<(), String> {
    if response.len() < 12 {
        return Err("DNS response is truncated".into());
    }
    if response[2] & 0x80 == 0 {
        return Err("DNS response is not marked as a response".into());
    }
    let rcode = response[3] & 0x0f;
    if rcode != 0 {
        return Err(format!("DNS response returned error code {rcode}"));
    }
    let questions = u16::from_be_bytes([response[4], response[5]]) as usize;
    let answers = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_dns_name(response, offset)?;
        if offset + 4 > response.len() {
            return Err("DNS response question is truncated".into());
        }
        offset += 4;
    }
    for _ in 0..answers {
        offset = skip_dns_name(response, offset)?;
        if offset + 10 > response.len() {
            return Err("DNS response answer is truncated".into());
        }
        let record_type = u16::from_be_bytes([response[offset], response[offset + 1]]);
        let class = u16::from_be_bytes([response[offset + 2], response[offset + 3]]);
        let length = u16::from_be_bytes([response[offset + 8], response[offset + 9]]) as usize;
        offset += 10;
        if offset + length > response.len() {
            return Err("DNS response answer data is truncated".into());
        }
        if record_type == 1 && class == 1 && length == 4 {
            let address = Ipv4Addr::new(
                response[offset],
                response[offset + 1],
                response[offset + 2],
                response[offset + 3],
            );
            if address == expected {
                return Ok(());
            }
        }
        offset += length;
    }
    Err(format!("DNS response did not contain A {expected}"))
}

fn skip_dns_name(response: &[u8], mut offset: usize) -> Result<usize, String> {
    loop {
        let length = *response
            .get(offset)
            .ok_or_else(|| "DNS response name is truncated".to_string())?;
        if length & 0xc0 == 0xc0 {
            if offset + 1 >= response.len() {
                return Err("DNS response compression pointer is truncated".into());
            }
            return Ok(offset + 2);
        }
        offset += 1;
        if length == 0 {
            return Ok(offset);
        }
        offset += length as usize;
        if offset > response.len() {
            return Err("DNS response name is truncated".into());
        }
    }
}

fn read_http_response(
    stream: &mut impl Read,
    max_body_bytes: usize,
) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("HTTP response read failed: {error}"))?;
        if read == 0 {
            return Err("HTTP response ended before its headers".into());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > MAX_HTTP_HEADER_BYTES {
            return Err("HTTP response headers exceeded 16 KiB".into());
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| "HTTP response status line is invalid".to_string())?;
    let content_length = headers.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .strip_prefix("content-length:")
            .and_then(|value| value.trim().parse::<usize>().ok())
    });
    if content_length.is_some_and(|length| length > max_body_bytes) {
        return Err(format!(
            "HTTP response body exceeded {max_body_bytes} bytes"
        ));
    }

    let mut body = buffer[header_end..].to_vec();
    if body.len() > max_body_bytes {
        return Err(format!(
            "HTTP response body exceeded {max_body_bytes} bytes"
        ));
    }
    if let Some(length) = content_length {
        while body.len() < length {
            let read = stream
                .read(&mut chunk)
                .map_err(|error| format!("HTTP response read failed: {error}"))?;
            if read == 0 {
                return Err("HTTP response body ended early".into());
            }
            body.extend_from_slice(&chunk[..read]);
            if body.len() > max_body_bytes || body.len() > length {
                return Err(format!(
                    "HTTP response body exceeded {max_body_bytes} bytes"
                ));
            }
        }
        body.truncate(length);
    } else {
        loop {
            let read = stream
                .read(&mut chunk)
                .map_err(|error| format!("HTTP response read failed: {error}"))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
            if body.len() > max_body_bytes {
                return Err(format!(
                    "HTTP response body exceeded {max_body_bytes} bytes"
                ));
            }
        }
    }
    Ok((status, body))
}

/// Windows AF_UNIX transport for the Caddy admin client.
///
/// The `windows` crate does not ship `sockaddr_un`, so it is defined here
/// per the OS layout. Timeouts are socket-level; no overlapped I/O needed.
#[cfg(windows)]
#[cfg(unix)]
mod admin_socket {
    use std::{
        io::{self, Read, Write},
        path::Path,
        sync::OnceLock,
        time::Duration,
    };
    use windows::{
        Win32::Networking::WinSock::{
            AF_UNIX, SEND_RECV_FLAGS, SO_RCVTIMEO, SO_SNDTIMEO, SOCK_STREAM, SOCKADDR, SOCKET,
            SOL_SOCKET, WSADATA, WSAGetLastError, WSAStartup, closesocket, connect, recv, send,
            setsockopt, socket as winsock_socket,
        },
        core::Error as WinError,
    };

    /// `sockaddr_un` as the OS lays it out.
    #[repr(C)]
    struct SockAddrUn {
        sun_family: u16,
        sun_path: [u8; 108],
    }

    fn ensure_wsa() -> Result<(), String> {
        static STARTED: OnceLock<Result<(), String>> = OnceLock::new();
        STARTED
            .get_or_init(|| unsafe {
                let mut data = std::mem::zeroed::<WSADATA>();
                if WSAStartup(0x202, &mut data) == 0 {
                    Ok(())
                } else {
                    Err(format!("WSAStartup failed: {}", WinError::from_thread()))
                }
            })
            .as_ref()
            .map(|_| ())
            .map_err(Clone::clone)
    }

    pub(super) struct WsaStream {
        handle: SOCKET,
    }

    impl Drop for WsaStream {
        fn drop(&mut self) {
            unsafe {
                closesocket(self.handle);
            }
        }
    }

    impl Read for WsaStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let done = unsafe { recv(self.handle, buf, SEND_RECV_FLAGS(0)) };
            if done > 0 {
                Ok(done as usize)
            } else if done == 0 {
                Ok(0)
            } else {
                Err(WinError::from_thread().into())
            }
        }
    }

    impl Write for WsaStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let done = unsafe { send(self.handle, buf, SEND_RECV_FLAGS(0)) };
            if done > 0 {
                Ok(done as usize)
            } else {
                Err(WinError::from_thread().into())
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn connect_admin(socket: &Path, timeout: Duration) -> Result<WsaStream, String> {
        ensure_wsa()?;
        let bytes = socket.as_os_str().as_encoded_bytes();
        if bytes.len() + 1 >= 108 {
            return Err("Caddy admin socket path is too long for AF_UNIX".into());
        }
        let mut addr = SockAddrUn {
            sun_family: AF_UNIX,
            sun_path: [0; 108],
        };
        addr.sun_path[..bytes.len()].copy_from_slice(bytes);
        let handle = unsafe { winsock_socket(AF_UNIX as i32, SOCK_STREAM, 0) }
            .map_err(|error| format!("unable to create admin socket: {error}"))?;
        // WinSock requires the full SOCKADDR_UN length here; the exact-length
        // form accepted on POSIX fails the connect on Windows.
        let connected = unsafe {
            connect(
                handle,
                &addr as *const SockAddrUn as *const SOCKADDR,
                std::mem::size_of::<SockAddrUn>() as i32,
            )
        };
        if connected != 0 {
            let code = unsafe { WSAGetLastError() };
            unsafe {
                closesocket(handle);
            }
            return Err(format!(
                "unable to connect to the Caddy admin socket: Winsock error {}",
                code.0
            ));
        }
        let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
        for option in [SO_RCVTIMEO, SO_SNDTIMEO] {
            let applied =
                unsafe { setsockopt(handle, SOL_SOCKET, option, Some(&millis.to_ne_bytes())) };
            if applied != 0 {
                unsafe {
                    closesocket(handle);
                }
                return Err(format!(
                    "unable to set the Caddy admin timeout: {}",
                    WinError::from_thread()
                ));
            }
        }
        Ok(WsaStream { handle })
    }
}
