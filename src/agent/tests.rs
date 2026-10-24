use super::*;

struct TestWorkload(ManagedWorkload);

impl Drop for TestWorkload {
    fn drop(&mut self) {
        self.0.runner.stop_all();
    }
}

fn app(name: &str, replicas: u32) -> TestWorkload {
    TestWorkload(ManagedWorkload::new(WorkloadSpec {
        name: name.into(),
        program: "/bin/sleep".into(),
        args: vec!["10".into()],
        replicas,
    }))
}

fn logged_app(name: &str, lines: &[String]) -> ManagedWorkload {
    let mut app = ManagedWorkload::new(WorkloadSpec {
        name: name.into(), program: "/usr/bin/printf".into(), args: vec!["%s\n".into()], replicas: 1,
    });
    app.spec.args.extend_from_slice(lines);
    app.runner.start(&app.spec).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.runner.logs_after(0).len() < lines.len() {
        if Instant::now() >= deadline {
            app.runner.stop_all();
            panic!("test app did not finish writing logs");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    app.runner.stop_all();
    app
}

#[test]
fn log_replies_page_through_lines_without_skipping_or_repeating_them() {
    let lines = vec!["x".repeat(4096); 8];
    let app = logged_app("logs", &lines);
    let mut after_id = 0;
    let mut received = Vec::new();
    loop {
        let reply = log_reply(&app.runner, after_id).unwrap();
        assert!(serde_json::to_vec(&reply).unwrap().len() + 1 <= tls::MAX_MESSAGE_BYTES as usize);
        let Response::Logs { entries } = reply else { panic!("expected logs") };
        if entries.is_empty() {
            break;
        }
        assert!(entries.len() < lines.len());
        for entry in entries {
            assert_eq!(entry.id, after_id + 1);
            after_id = entry.id;
            received.push(entry.text);
        }
    }
    assert_eq!(received, lines);
}

#[test]
fn escaped_large_lines_fit_without_changing_stored_logs() {
    let original = "\u{1}".repeat(4096);
    let app = logged_app("logs", &[original.clone(), "next".into()]);
    let reply = log_reply(&app.runner, 0).unwrap();
    let mut bytes = serde_json::to_vec(&reply).unwrap();
    bytes.push(b'\n');
    assert!(bytes.len() <= tls::MAX_MESSAGE_BYTES as usize);
    assert!(tls::read_message(&mut bytes.as_slice()).is_ok());
    let Response::Logs { entries } = reply else { panic!("expected logs") };
    assert_eq!(entries[0].id, 1);
    assert!(entries[0].text.ends_with(" [truncated]"));
    assert!(entries[0].text.len() < original.len());
    assert_eq!(app.runner.logs_after(0)[0].text, original);

    let Response::Logs { entries } = log_reply(&app.runner, 1).unwrap() else { panic!("expected logs") };
    assert_eq!(entries[0].text, "next");
    assert!(matches!(log_reply(&app.runner, 2).unwrap(), Response::Logs { entries } if entries.is_empty()));
}

#[test]
fn named_log_requests_require_auth_and_never_fall_back_to_another_app() {
    let workloads = &mut [logged_app("api", &["api output".into()]), logged_app("worker", &["worker output".into()])];
    for name in ["api", "worker"] {
        let request = Request::LogsApp { workload_name: name.into(), after_id: 0 };
        let Response::Logs { entries } = reply_for(workloads, "test-token", request) else { panic!("expected logs") };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, format!("{name} output"));
    }
    for (token, name, expected) in [
        ("test-token", "missing", "Unknown app: missing"),
        ("wrong-token", "missing", "Authentication failed"),
        ("wrong-token", "api", "Authentication failed"),
    ] {
        let request = Request::LogsApp { workload_name: name.into(), after_id: 0 };
        let Response::Error { message } = reply_for(workloads, token, request) else { panic!("expected an error") };
        assert_eq!(message, expected);
    }
    let request = Request::LogsApp { workload_name: "worker".into(), after_id: 1 };
    assert!(matches!(reply_for(workloads, "test-token", request), Response::Logs { entries } if entries.is_empty()));
    assert_eq!(workloads[0].spec.replicas, 1);
    assert_eq!(workloads[1].spec.replicas, 1);
}

#[test]
fn construction_keeps_settings_without_starting_processes() {
    let before = Instant::now();
    let mut app = app("api", 2);
    assert_eq!(app.0.spec.name, "api");
    assert_eq!(app.0.spec.replicas, 2);
    assert_eq!(app.0.runner.running_count(), 0);
    assert!(app.0.retry_at >= before && app.0.retry_at <= Instant::now());
}

#[test]
fn ticks_start_missing_copies_and_stop_extra_copies() {
    let keep_alive = AtomicBool::new(true);
    let mut app = app("api", 2);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 2);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 2);
    app.0.spec.replicas = 1;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 1);
    app.0.spec.replicas = 0;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 0);
}

#[test]
fn stopping_and_restarting_one_app_does_not_change_another() {
    let keep_alive = AtomicBool::new(true);
    let mut first = app("api", 2);
    let mut second = app("scraper", 1);
    first.0.tick(&keep_alive);
    second.0.tick(&keep_alive);
    let stopped = first.0.stop();
    assert_eq!(stopped.desired, 0);
    assert_eq!(stopped.running, 0);
    first.0.tick(&keep_alive);
    assert_eq!(first.0.runner.running_count(), 0);
    assert_eq!(second.0.runner.running_count(), 1);
    first.0.scale(1).unwrap();
    first.0.tick(&keep_alive);
    assert_eq!(first.0.runner.running_count(), 1);
    assert_eq!(second.0.runner.running_count(), 1);
    assert_eq!(second.0.spec.replicas, 1);
}

#[test]
fn status_and_scale_report_current_counts_before_the_next_tick() {
    let keep_alive = AtomicBool::new(true);
    let mut app = app("api", 2);
    let initial = app.0.status();
    assert_eq!(initial.workload_name.as_deref(), Some("api"));
    assert_eq!(initial.desired, 2);
    assert_eq!(initial.running, 0);
    app.0.tick(&keep_alive);
    let scaled = app.0.scale(1).unwrap();
    assert_eq!(scaled.desired, 1);
    assert_eq!(scaled.running, 2);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.status().running, 1);
}

#[test]
fn scale_accepts_boundaries_and_rejects_invalid_counts_without_changes() {
    let mut app = app("api", 1);
    assert_eq!(app.0.scale(0).unwrap().desired, 0);
    assert_eq!(app.0.scale(MAX_REPLICAS).unwrap().desired, MAX_REPLICAS);
    let retry_at = app.0.retry_at;
    let error = app.0.scale(MAX_REPLICAS + 1).unwrap_err();
    assert_eq!(
        error,
        format!("Replica count cannot exceed {}", MAX_REPLICAS)
    );
    assert_eq!(app.0.spec.replicas, MAX_REPLICAS);
    assert_eq!(app.0.status().running, 0);
    assert_eq!(app.0.retry_at, retry_at);
}

#[test]
fn failed_launch_waits_five_seconds_without_delaying_another_app() {
    let keep_alive = AtomicBool::new(true);
    let directory = tempfile::tempdir().unwrap();
    let mut failed = app("broken", 1);
    failed.0.spec.program = directory
        .path()
        .join("missing-program")
        .to_str()
        .unwrap()
        .into();
    let before = Instant::now();
    failed.0.tick(&keep_alive);
    assert_eq!(failed.0.runner.running_count(), 0);
    assert!(failed.0.retry_at >= before + Duration::from_secs(5));
    assert!(failed.0.retry_at <= Instant::now() + Duration::from_secs(5));

    // a fixed app must still wait to retry
    failed.0.spec.program = "/bin/sleep".into();
    failed.0.tick(&keep_alive);
    assert_eq!(failed.0.runner.running_count(), 0);
    let mut healthy = app("healthy", 1);
    healthy.0.tick(&keep_alive);
    assert_eq!(healthy.0.runner.running_count(), 1);

    // skip the wait in this test
    failed.0.retry_at = Instant::now();
    failed.0.tick(&keep_alive);
    assert_eq!(failed.0.runner.running_count(), 1);
    assert_eq!(healthy.0.runner.running_count(), 1);
}

#[test]
fn launch_delay_does_not_prevent_stopping_extra_copies() {
    let keep_alive = AtomicBool::new(true);
    let mut app = app("api", 2);
    app.0.tick(&keep_alive);
    app.0.retry_at = Instant::now() + Duration::from_secs(60);
    app.0.spec.replicas = 0;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 0);
}

#[test]
fn shutdown_flag_prevents_start_and_stop_actions() {
    let keep_alive = AtomicBool::new(false);
    let mut app = app("api", 1);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 0);
    keep_alive.store(true, Ordering::SeqCst);
    app.0.tick(&keep_alive);
    keep_alive.store(false, Ordering::SeqCst);
    app.0.spec.replicas = 0;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 1);
}

fn reply_for(workloads: &mut [ManagedWorkload], token: &str, request: Request) -> Response {
    let server = DeviceIdentity::generate().unwrap();
    let client = DeviceIdentity::generate().unwrap();
    let mut store = TrustStore::new();
    store.add(crate::trust::TrustedPeer {
        name: "client".into(), public_key: client.public_key_pem(),
    }).unwrap();
    let server_config = Arc::new(device_tls::server_config(&server, &store).unwrap());
    let peer = crate::trust::TrustedPeer {
        name: "server".into(), public_key: server.public_key_pem(),
    };
    let client_config = Arc::new(device_tls::client_config(&client, &peer).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap().to_string();

    std::thread::scope(|scope| {
        let client = scope.spawn(move || {
            let mut stream = tls::connect(&address, "localhost", client_config).unwrap();
            let request = AuthenticatedRequest { token: token.into(), request };
            serde_json::to_writer(&mut stream, &request).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
            serde_json::from_str(&tls::read_message(&mut stream).unwrap()).unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "test client did not connect");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("test listener failed: {error}"),
            }
        };
        let mut stream = tls::accept(socket, server_config).unwrap();
        handle_client(&mut stream, workloads, "test-token", &mut Sampler::new()).unwrap();
        client.join().unwrap()
    })
}

#[test]
fn named_status_selects_the_app_and_keeps_old_commands_safe() {
    let manifest: AgentManifest = serde_json::from_str(include_str!("../../examples/apps.json")).unwrap();
    let mut workloads = Vec::new();
    for spec in manifest.workloads {
        workloads.push(ManagedWorkload::new(spec));
    }

    for (name, desired) in [("demo-fast", 1), ("demo-slow", 2)] {
        let request = Request::StatusApp { workload_name: name.into() };
        match reply_for(&mut workloads, "test-token", request) {
            Response::Status { status } => {
                assert_eq!(status.workload_name.as_deref(), Some(name));
                assert_eq!(status.desired, desired);
                assert_eq!(status.running, 0);
                let metrics = status.metrics.expect("agent replies should include host readings");
                assert!(metrics.total_mem > 0);
                assert!(metrics.used_mem <= metrics.total_mem);
            }
            reply => panic!("expected named status: {reply:?}"),
        }
    }

    for (token, request, expected) in [
        ("test-token", Request::StatusApp { workload_name: "missing".into() }, "Unknown app: missing"),
        ("test-token", Request::StatusApp { workload_name: "".into() }, "Unknown app: "),
        ("wrong-token", Request::StatusApp { workload_name: "missing".into() }, "Authentication failed"),
        ("test-token", Request::Status, "Choose an app before sending commands"),
        ("test-token", Request::Stop, "Choose an app before sending commands"),
        ("test-token", Request::Scale { replicas: 0 }, "Choose an app before sending commands"),
    ] {
        match reply_for(&mut workloads, token, request) {
            Response::Error { message } => assert_eq!(message, expected),
            reply => panic!("expected an error: {reply:?}"),
        }
    }
    assert_eq!(workloads[0].spec.replicas, 1);
    assert_eq!(workloads[1].spec.replicas, 2);

    workloads.truncate(1);
    match reply_for(&mut workloads, "test-token", Request::Status) {
        Response::Status { status } => assert_eq!(status.workload_name.as_deref(), Some("demo-fast")),
        reply => panic!("expected old single-app status: {reply:?}"),
    }
    let request = Request::StatusApp { workload_name: "demo-slow".into() };
    match reply_for(&mut workloads, "test-token", request) {
        Response::Error { message } => assert_eq!(message, "Unknown app: demo-slow"),
        reply => panic!("must not fall back to the first app: {reply:?}"),
    }
}

#[test]
fn named_controls_only_change_the_selected_app() {
    let manifest: AgentManifest = serde_json::from_str(include_str!("../../examples/apps.json")).unwrap();
    let mut workloads: Vec<_> = manifest.workloads.into_iter().map(ManagedWorkload::new).collect();
    let request = Request::ScaleApp { workload_name: "demo-fast".into(), replicas: 3 };
    match reply_for(&mut workloads, "test-token", request) {
        Response::Status { status } => {
            assert_eq!(status.workload_name.as_deref(), Some("demo-fast"));
            assert_eq!(status.desired, 3);
        }
        reply => panic!("expected scale status: {reply:?}"),
    }
    assert_eq!(workloads[1].spec.replicas, 2);

    let request = Request::StopApp { workload_name: "demo-slow".into() };
    match reply_for(&mut workloads, "test-token", request) {
        Response::Status { status } => {
            assert_eq!(status.workload_name.as_deref(), Some("demo-slow"));
            assert_eq!(status.desired, 0);
            assert_eq!(status.running, 0);
        }
        reply => panic!("expected stop status: {reply:?}"),
    }
    assert_eq!(workloads[0].spec.replicas, 3);

    for (token, request) in [
        ("test-token", Request::StopApp { workload_name: "missing".into() }),
        ("test-token", Request::ScaleApp { workload_name: "missing".into(), replicas: 0 }),
        ("test-token", Request::ScaleApp { workload_name: "demo-fast".into(), replicas: MAX_REPLICAS + 1 }),
        ("wrong-token", Request::StopApp { workload_name: "demo-fast".into() }),
        ("wrong-token", Request::ScaleApp { workload_name: "demo-fast".into(), replicas: 0 }),
    ] {
        assert!(matches!(reply_for(&mut workloads, token, request), Response::Error { .. }));
        assert_eq!(workloads[0].spec.replicas, 3);
        assert_eq!(workloads[1].spec.replicas, 0);
    }
}

#[test]
fn listing_apps_requires_auth_and_keeps_their_targets() {
    let manifest: AgentManifest = serde_json::from_str(include_str!("../../examples/apps.json")).unwrap();
    let mut workloads: Vec<_> = manifest.workloads.into_iter().map(ManagedWorkload::new).collect();
    match reply_for(&mut workloads, "wrong-token", Request::ListApps) {
        Response::Error { message } => assert_eq!(message, "Authentication failed"),
        reply => panic!("must authenticate before listing apps: {reply:?}"),
    }
    match reply_for(&mut workloads, "test-token", Request::ListApps) {
        Response::Apps { apps } => {
            assert_eq!(apps.len(), 2);
            for (status, (name, desired)) in apps.iter().zip([("demo-fast", 1), ("demo-slow", 2)]) {
                assert_eq!(status.workload_name.as_deref(), Some(name));
                assert_eq!(status.desired, desired);
                assert_eq!(status.running, 0);
                assert!(status.metrics.is_some());
            }
        }
        reply => panic!("expected an app list: {reply:?}"),
    }
    assert_eq!(workloads[0].spec.replicas, 1);
    assert_eq!(workloads[1].spec.replicas, 2);
}
