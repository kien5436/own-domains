use own_domains_client::{Client, ClientError};
use own_domains_protocol::{CommandResult, PROTOCOL_VERSION, ResponseEnvelope};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    thread,
    time::{Duration, Instant},
};

fn server(frames: Vec<(Vec<u8>, Duration)>) -> (std::path::PathBuf, thread::JoinHandle<()>) {
    let path = std::env::temp_dir().join(format!("od-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let runner = thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for _ in 0..if index == 0 { 1 } else { 2 } {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: own_domains_protocol::RequestEnvelope =
                    serde_json::from_str(&line).unwrap();
                let result = if matches!(
                    request.command,
                    own_domains_protocol::Command::Subscribe { .. }
                ) {
                    CommandResult::Subscribed
                } else {
                    CommandResult::Handshake {
                        protocol_version: PROTOCOL_VERSION,
                    }
                };
                writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(&ResponseEnvelope::ok(request.request_id, result))
                        .unwrap()
                )
                .unwrap();
            }
            if index == 1 {
                for (bytes, delay) in &frames {
                    thread::sleep(*delay);
                    if stream.write_all(bytes).is_err() {
                        break;
                    }
                }
            }
        }
    });
    (path, runner)
}

#[test]
fn subscription_preserves_partial_frames_across_idle_timeouts() {
    let (path, runner) = server(vec![
        (
            format!(
                "{{\"protocolVersion\":{PROTOCOL_VERSION},\"sequence\":1,\"event\":{{\"type\":\"settings"
            )
            .into_bytes(),
            Duration::ZERO,
        ),
        (
            b"Changed\",\"payload\":{\"revision\":1}}}\n".to_vec(),
            Duration::from_millis(300),
        ),
    ]);
    let client = Client::with_timeout(&path, Duration::from_millis(100)).unwrap();
    let mut stream = client.subscribe(None).unwrap();
    assert_eq!(stream.recv().unwrap().sequence, 1);
    runner.join().unwrap();
}
#[test]
fn cancelling_idle_subscription_interrupts_reader() {
    let (path, runner) = server(vec![(vec![], Duration::from_millis(150))]);
    let mut stream = Client::connect(&path).unwrap().subscribe(None).unwrap();
    let cancel = stream.cancellation_handle().unwrap();
    let reader = thread::spawn(move || stream.recv());
    let start = Instant::now();
    cancel.cancel().unwrap();
    assert!(reader.join().unwrap().is_err());
    assert!(start.elapsed() < Duration::from_millis(100));
    runner.join().unwrap();
}
#[test]
fn rejects_incompatible_event_versions() {
    let (path, runner) = server(vec![(b"{\"protocolVersion\":3,\"sequence\":1,\"event\":{\"type\":\"settingsChanged\",\"payload\":{\"revision\":1}}}\n".to_vec(), Duration::ZERO)]);
    let mut stream = Client::connect(&path).unwrap().subscribe(None).unwrap();
    assert!(matches!(
        stream.recv(),
        Err(ClientError::IncompatibleVersion { received: 3, .. })
    ));
    runner.join().unwrap();
}
