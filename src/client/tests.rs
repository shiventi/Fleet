use super::*;
use crate::runner::LogStream;
use crate::trust::TrustStore;
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn with_reply<T>(expected: Request, reply: Response, call: impl FnOnce(&FleetClient, &str) -> io::Result<T>) -> io::Result<T> {
    let server = DeviceIdentity::generate().unwrap();
    let identity = DeviceIdentity::generate().unwrap();
    let mut store = TrustStore::new();
    store.add(TrustedPeer { name: "client".into(), public_key: identity.public_key_pem() }).unwrap();
    let config = Arc::new(device_tls::server_config(&server, &store).unwrap());
    let peer = TrustedPeer { name: "server".into(), public_key: server.public_key_pem() };
    let client = FleetClient {
        token: "test-token".into(), config: Arc::new(device_tls::client_config(&identity, &peer).unwrap()),
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap().to_string();

    std::thread::scope(|scope| {
        let server = scope.spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "client did not connect");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("test listener failed: {error}"),
                }
            };
            let mut stream = tls::accept(socket, config).unwrap();
            let input = tls::read_message(&mut stream).unwrap();
            let request: AuthenticatedRequest = serde_json::from_str(&input).unwrap();
            assert_eq!(request.token, "test-token");
            assert_eq!(serde_json::to_value(request.request).unwrap(), serde_json::to_value(expected).unwrap());
            serde_json::to_writer(&mut stream, &reply).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        });
        let result = call(&client, &address);
        server.join().unwrap();
        result
    })
}

#[test]
fn fetch_logs_sends_the_app_and_cursor_and_reads_the_reply() {
    let entries = vec![LogEntry { id: 8, pid: 42, stream: LogStream::Stderr, text: "hello".into() }];
    let request = Request::LogsApp { workload_name: "worker".into(), after_id: 7 };
    let logs = with_reply(request, Response::Logs { entries }, |client, address| {
        client.fetch_logs(address, "worker", 7, "localhost")
    }).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].id, 8);
    assert_eq!(logs[0].pid, 42);
    assert!(matches!(logs[0].stream, LogStream::Stderr));
    assert_eq!(logs[0].text, "hello");
}

#[test]
fn fetch_logs_handles_empty_replies_errors_and_wrong_reply_types() {
    for (reply, expected) in [
        (Response::Logs { entries: Vec::new() }, None),
        (Response::Error { message: "Authentication failed".into() }, Some(io::ErrorKind::Other)),
        (Response::Apps { apps: Vec::new() }, Some(io::ErrorKind::InvalidData)),
        (Response::Status { status: StatusResponse { workload_name: Some("worker".into()), desired: 0, running: 0, metrics: None } }, Some(io::ErrorKind::InvalidData)),
    ] {
        let request = Request::LogsApp { workload_name: "worker".into(), after_id: 7 };
        let result = with_reply(request, reply, |client, address| client.fetch_logs(address, "worker", 7, "localhost"));
        match expected {
            None => assert!(result.unwrap().is_empty()),
            Some(kind) => assert_eq!(result.unwrap_err().kind(), kind),
        }
    }
}

#[test]
fn other_client_calls_reject_log_replies() {
    for request in [Request::Status, Request::ListApps, Request::Stop, Request::Scale { replicas: 1 }, Request::ScaleApp { workload_name: "worker".into(), replicas: 1 }] {
        let command = serde_json::to_value(&request).unwrap()["command"].as_str().unwrap().to_owned();
        let error = with_reply(request, Response::Logs { entries: Vec::new() }, |client, address| {
            match command.as_str() {
                "status" => client.fetch_status(address, "localhost").map(|_| ()),
                "list_apps" => client.list_apps(address, "localhost").map(|_| ()),
                "stop" => client.stop(address, "localhost"),
                "scale" => client.scale(address, 1, "localhost").map(|_| ()),
                "scale_app" => client.scale_app(address, "worker", 1, "localhost").map(|_| ()),
                _ => unreachable!(),
            }
        }).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn prints_streams_and_escapes_terminal_controls() {
    let entries = vec![
        LogEntry { id: 3, pid: 42, stream: LogStream::Stdout, text: "hello".into() },
        LogEntry { id: 4, pid: 43, stream: LogStream::Stderr, text: "\x1b[2Jwarning\r\t\u{e9}".into() },
    ];
    let mut output = Vec::new();
    let mut after = 2;
    print_logs(&mut output, &entries, &mut after).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("[3 pid=42 stdout] hello\n"));
    assert!(text.contains("[4 pid=43 stderr] \\u{1b}[2Jwarning\\r\\t\u{e9}\n"));
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\r'));
    assert_eq!(after, 4);
}

#[test]
fn refuses_old_or_out_of_order_log_ids_before_printing() {
    for ids in [[2, 3], [4, 3], [3, 3]] {
        let entries: Vec<_> = ids.into_iter().map(|id| LogEntry {
            id, pid: 42, stream: LogStream::Stdout, text: "line".into(),
        }).collect();
        let mut output = Vec::new();
        let mut after = 2;
        let error = print_logs(&mut output, &entries, &mut after).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(output.is_empty());
        assert_eq!(after, 2);
    }
}
