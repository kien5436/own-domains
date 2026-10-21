use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_HEADER_BYTES: usize = 8 * 1024;
const REQUEST_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum SetupServerError {
    NonLoopbackAddress(SocketAddr),
    Bind(io::Error),
    ThreadSpawn(io::Error),
    ThreadPanicked,
}

impl fmt::Display for SetupServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonLoopbackAddress(address) => write!(
                formatter,
                "setup server requires a loopback address, got {address}"
            ),
            Self::Bind(error) => write!(formatter, "could not bind setup server: {error}"),
            Self::ThreadSpawn(error) => write!(formatter, "could not start setup server: {error}"),
            Self::ThreadPanicked => write!(formatter, "setup server thread panicked"),
        }
    }
}

impl std::error::Error for SetupServerError {}

pub struct SetupServer {
    address: SocketAddr,
    shutting_down: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SetupServer {
    pub fn bind(address: SocketAddr, certificate_path: PathBuf) -> Result<Self, SetupServerError> {
        if !address.ip().is_loopback() {
            return Err(SetupServerError::NonLoopbackAddress(address));
        }
        let listener = TcpListener::bind(address).map_err(SetupServerError::Bind)?;
        let address = listener.local_addr().map_err(SetupServerError::Bind)?;
        listener
            .set_nonblocking(true)
            .map_err(SetupServerError::Bind)?;
        let shutting_down = Arc::new(AtomicBool::new(false));
        let cancellation = Arc::clone(&shutting_down);
        let thread = thread::Builder::new()
            .name("own-domains-setup-server".into())
            .spawn(move || serve(listener, certificate_path, cancellation))
            .map_err(SetupServerError::ThreadSpawn)?;
        Ok(Self {
            address,
            shutting_down,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn shutdown(&self) -> Result<(), SetupServerError> {
        self.shutting_down.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(50));
        if let Some(thread) = self
            .thread
            .lock()
            .expect("setup server thread mutex poisoned")
            .take()
        {
            thread
                .join()
                .map_err(|_| SetupServerError::ThreadPanicked)?;
        }
        Ok(())
    }
}

impl Drop for SetupServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn serve(listener: TcpListener, certificate_path: PathBuf, shutting_down: Arc<AtomicBool>) {
    while !shutting_down.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle_connection(stream, &certificate_path, &shutting_down),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(mut stream: TcpStream, certificate_path: &Path, shutting_down: &AtomicBool) {
    let response = match read_request(&mut stream, shutting_down) {
        Ok(request) => response_for(&request, certificate_path),
        Err(ReadRequestError::TooLarge) => Response::new(
            431,
            "Request Header Fields Too Large",
            "text/plain; charset=utf-8",
            b"header too large\n".to_vec(),
        ),
        Err(ReadRequestError::Timeout) => Response::new(
            408,
            "Request Timeout",
            "text/plain; charset=utf-8",
            b"request timeout\n".to_vec(),
        ),
        Err(ReadRequestError::Invalid) => Response::new(
            400,
            "Bad Request",
            "text/plain; charset=utf-8",
            b"bad request\n".to_vec(),
        ),
    };
    let _ = stream.write_all(&response.encode());
}

enum ReadRequestError {
    TooLarge,
    Timeout,
    Invalid,
}
struct Request {
    method: String,
    path: String,
}

fn read_request(
    stream: &mut TcpStream,
    shutting_down: &AtomicBool,
) -> Result<Request, ReadRequestError> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| ReadRequestError::Invalid)?;
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let mut header = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        if shutting_down.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(ReadRequestError::Timeout);
        }
        match stream.read(&mut buffer) {
            Ok(0) => return Err(ReadRequestError::Invalid),
            Ok(count) => {
                header.extend_from_slice(&buffer[..count]);
                if header.len() > MAX_HEADER_BYTES {
                    return Err(ReadRequestError::TooLarge);
                }
                if header.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => return Err(ReadRequestError::Invalid),
        }
    }
    let text = std::str::from_utf8(&header).map_err(|_| ReadRequestError::Invalid)?;
    let line = text.split("\r\n").next().ok_or(ReadRequestError::Invalid)?;
    let mut pieces = line.split_ascii_whitespace();
    let method = pieces.next().ok_or(ReadRequestError::Invalid)?;
    let path = pieces.next().ok_or(ReadRequestError::Invalid)?;
    if pieces.next() != Some("HTTP/1.1") || pieces.next().is_some() || !path.starts_with('/') {
        return Err(ReadRequestError::Invalid);
    }
    Ok(Request {
        method: method.into(),
        path: path.into(),
    })
}

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    head_only: bool,
    extra_headers: Vec<(&'static str, &'static str)>,
}
impl Response {
    fn new(status: u16, reason: &'static str, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason,
            content_type,
            body,
            head_only: false,
            extra_headers: vec![],
        }
    }
    fn encode(&self) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n", self.status, self.reason, self.content_type, self.body.len()).into_bytes();
        for (name, value) in &self.extra_headers {
            response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        response.extend_from_slice(b"\r\n");
        if !self.head_only {
            response.extend_from_slice(&self.body);
        }
        response
    }
}

fn response_for(request: &Request, certificate_path: &Path) -> Response {
    if request.method != "GET" && request.method != "HEAD" {
        let mut response = Response::new(
            405,
            "Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed\n".to_vec(),
        );
        response.extra_headers.push(("Allow", "GET, HEAD"));
        return response;
    }
    let certificate = || read_certificate(certificate_path);
    let result = match request.path.as_str() {
        "/" => Ok(("text/html; charset=utf-8", b"<!doctype html><html><body><h1>Own Domains</h1><p>Install the local certificate to continue.</p></body></html>".to_vec())),
        "/ca.crt" => certificate().map(|certificate| ("application/x-x509-ca-cert", certificate.pem)),
        "/ca.mobileconfig" => certificate().map(|certificate| ("application/x-apple-aspen-config", mobileconfig(&certificate.der))),
        "/fingerprint" => certificate().map(|certificate| ("text/plain; charset=utf-8", format!("SHA256:{:X}\n", Sha256::digest(&certificate.der)).into_bytes())),
        _ => return Response::new(404, "Not Found", "text/plain; charset=utf-8", b"not found\n".to_vec()),
    };
    match result {
        Ok((content_type, body)) => {
            let mut response = Response::new(200, "OK", content_type, body);
            response.head_only = request.method == "HEAD";
            response
        }
        Err(()) => Response::new(
            404,
            "Not Found",
            "text/plain; charset=utf-8",
            b"certificate unavailable\n".to_vec(),
        ),
    }
}

struct Certificate {
    pem: Vec<u8>,
    der: Vec<u8>,
}
fn read_certificate(path: &Path) -> Result<Certificate, ()> {
    let pem = fs::read(path).map_err(|_| ())?;
    let text = std::str::from_utf8(&pem).map_err(|_| ())?;
    let content = text
        .strip_prefix("-----BEGIN CERTIFICATE-----")
        .and_then(|value| {
            value
                .strip_suffix("-----END CERTIFICATE-----\n")
                .or_else(|| value.strip_suffix("-----END CERTIFICATE-----"))
        })
        .ok_or(())?;
    let encoded: String = content
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    let der = STANDARD.decode(encoded).map_err(|_| ())?;
    if der.is_empty() {
        return Err(());
    }
    Ok(Certificate { pem, der })
}

fn mobileconfig(certificate: &[u8]) -> Vec<u8> {
    let payload_uuid = stable_uuid(b"own-domains-ca-payload", certificate);
    let profile_uuid = stable_uuid(b"own-domains-ca-profile", certificate);
    let certificate = STANDARD.encode(certificate);
    format!(r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>PayloadContent</key><array><dict><key>PayloadType</key><string>com.apple.security.root</string><key>PayloadVersion</key><integer>1</integer><key>PayloadIdentifier</key><string>dev.owndomains.ca</string><key>PayloadUUID</key><string>{payload_uuid}</string><key>PayloadContent</key><data>{certificate}</data></dict></array><key>PayloadType</key><string>Configuration</string><key>PayloadVersion</key><integer>1</integer><key>PayloadIdentifier</key><string>dev.owndomains.profile</string><key>PayloadUUID</key><string>{profile_uuid}</string><key>PayloadDisplayName</key><string>Own Domains Certificate</string></dict></plist>"#).into_bytes()
}

fn stable_uuid(namespace: &[u8], certificate: &[u8]) -> String {
    let digest = Sha256::new()
        .chain_update(namespace)
        .chain_update(certificate)
        .finalize();
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        digest[0],
        digest[1],
        digest[2],
        digest[3],
        digest[4],
        digest[5],
        digest[6],
        digest[7],
        digest[8],
        digest[9],
        digest[10],
        digest[11],
        digest[12],
        digest[13],
        digest[14],
        digest[15]
    )
}
