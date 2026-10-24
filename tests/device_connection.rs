#![cfg(unix)]

use rustix::process::{Pid, Signal, kill_process, test_kill_process};
use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "local-integration-test-token";

fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fleet"));
    command.env("HOME", home).env("FLEET_TOKEN", TOKEN);
    command
}

fn run(home: &Path, args: &[&str]) -> Output {
    command(home).args(args).output().expect("run Fleet")
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("text output")
}

fn device(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let home = root.join(name);
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    let public = root.join(format!("{name}.pem"));
    fs::write(&public, success(run(&home, &["identity"]))).unwrap();
    (home, public)
}

fn approve(home: &Path, name: &str, public: &Path) {
    success(run(home, &["peers", "add", name, public.to_str().unwrap()]));
}

fn wait_for_counts(home: &Path, address: &str, count: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = success(run(
            home,
            &["status-device", "--address", address, "--peer", "server"],
        ));
        if status.contains(&format!("Desired copies: {count}\n"))
            && status.contains(&format!("Running copies: {count}\n"))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "counts did not reach {count}: {status}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Agent {
    child: Child,
    log: PathBuf,
}

impl Agent {
    fn start(home: &Path, manifest: &Path, log: PathBuf) -> (Self, String) {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap().to_string();
        drop(reservation);
        let output = fs::File::create(&log).unwrap();
        let mut agent = Self {
            child: command(home)
                .args([
                    "agent-device",
                    "--listen",
                    &address,
                    "--manifest",
                    manifest.to_str().unwrap(),
                ])
                .stdout(Stdio::from(output.try_clone().unwrap()))
                .stderr(Stdio::from(output))
                .spawn()
                .expect("start local agent"),
            log,
        };
        agent.ready();
        (agent, address)
    }

    fn ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let log = fs::read_to_string(&self.log).unwrap();
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "agent exited: {log}"
            );
            if log.contains("Started process:") {
                return;
            }
            assert!(Instant::now() < deadline, "agent startup timed out: {log}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stop(&mut self) -> std::process::ExitStatus {
        if let Some(status) = self.child.try_wait().unwrap() {
            return status;
        }
        kill_process(Pid::from_child(&self.child), Signal::INT).expect("send Ctrl+C");
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("agent did not stop after Ctrl+C");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_apps(&mut self, expected: &[(&str, usize)], timeout: Duration) -> Vec<(String, Pid)> {
        let deadline = Instant::now() + timeout;
        loop {
            let log = fs::read_to_string(&self.log).unwrap();
            assert!(self.child.try_wait().unwrap().is_none(), "agent exited: {log}");
            let mut name = String::new();
            let mut launches = Vec::new();
            for line in log.lines() {
                if let Some(app) = line.strip_prefix("Starting workload: ") {
                    name = app.to_string();
                } else if let Some(pid) = line.strip_prefix("Started process: ") {
                    launches.push((name.clone(), Pid::from_raw(pid.parse().unwrap()).unwrap()));
                }
            }
            if expected.iter().all(|(name, count)| launches.iter().filter(|(app, _)| app.as_str() == *name).count() >= *count) {
                return launches;
            }
            assert!(Instant::now() < deadline, "apps did not start or restart: {log}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = kill_process(Pid::from_child(&self.child), Signal::INT);
        let deadline = Instant::now() + Duration::from_secs(7);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn local_run_still_prints_stdout_and_stderr() {
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("local.json");
    fs::write(&manifest, r#"{"name":"local-test","program":"/bin/sh","args":["-c","printf 'local-output\n'; printf 'local-error\n' >&2; sleep 60"],"replicas":1}"#).unwrap();
    let log = root.path().join("run.log");
    let output = fs::File::create(&log).unwrap();
    let mut local = Agent {
        child: command(root.path()).args(["run", manifest.to_str().unwrap()])
            .stdout(output.try_clone().unwrap()).stderr(output).spawn().unwrap(),
        log,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = fs::read_to_string(&local.log).unwrap();
        assert!(local.child.try_wait().unwrap().is_none(), "run exited early: {text}");
        if text.contains("stdout] local-output") && text.contains("stderr] local-error") {
            break;
        }
        assert!(Instant::now() < deadline, "local output was lost: {text}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(local.stop().success());
}

#[test]
fn log_cli_selects_an_app_follows_new_lines_and_stops_without_stopping_the_agent() {
    let root = tempfile::tempdir().unwrap();
    let (server, server_public) = device(root.path(), "server");
    let (client, client_public) = device(root.path(), "client");
    approve(&server, "client", &client_public);
    approve(&client, "server", &server_public);
    let manifest = root.path().join("logs.json");
    fs::write(&manifest, serde_json::to_vec(&serde_json::json!({ "workloads": [
        { "name": "chatty", "program": "/bin/sh", "args": ["-c", "printf 'ready\\n'; printf '\\033[2Jwarning\\n' >&2; i=0; while [ \"$i\" -lt 60 ]; do i=$((i+1)); printf 'tick %s\\n' \"$i\"; sleep 1; done"], "replicas": 1 },
        { "name": "other", "program": "/bin/sh", "args": ["-c", "printf 'other-app\\n'; sleep 60"], "replicas": 1 }
    ] })).unwrap()).unwrap();
    let (mut agent, address) = Agent::start(&server, &manifest, root.path().join("agent.log"));
    let args = ["logs-device", "--address", &address, "--peer", "server", "--app", "chatty"];
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        let text = success(run(&client, &args));
        if text.contains("stdout] ready") && text.contains("stderr]") && text.contains("tick ") {
            break text;
        }
        assert!(Instant::now() < deadline, "logs did not arrive: {text}");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(snapshot.contains("\\u{1b}[2Jwarning"));
    assert!(!snapshot.contains('\x1b'));
    assert!(!snapshot.contains("other-app"));
    let after: u64 = snapshot.lines().last().unwrap().trim_start_matches('[').split_whitespace().next().unwrap().parse().unwrap();

    let mut resume_args = args.to_vec();
    let max_id = u64::MAX.to_string();
    resume_args.extend(["--after", &max_id]);
    assert!(success(run(&client, &resume_args)).is_empty());
    let bad = command(&client).env("FLEET_TOKEN", "wrong-token").args(args).output().unwrap();
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("Authentication failed"));

    let log = root.path().join("follow.log");
    let output = fs::File::create(&log).unwrap();
    let mut follower = Agent {
        child: command(&client).args(args).args(["--after", &after.to_string(), "--follow"])
            .stdout(output.try_clone().unwrap()).stderr(output).spawn().unwrap(),
        log,
    };
    let deadline = Instant::now() + Duration::from_secs(7);
    loop {
        let text = fs::read_to_string(&follower.log).unwrap();
        assert!(follower.child.try_wait().unwrap().is_none(), "follower exited early: {text}");
        if text.contains("stdout] tick ") {
            break;
        }
        assert!(Instant::now() < deadline, "follow did not print new logs: {text}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(follower.stop().success());
    let output = fs::read_to_string(&follower.log).unwrap();
    let mut previous = after;
    for line in output.lines() {
        let id: u64 = line.trim_start_matches('[').split_whitespace().next().unwrap().parse().unwrap();
        assert!(id > previous, "repeated or old line: {line}");
        previous = id;
    }
    assert!(agent.child.try_wait().unwrap().is_none());
    assert!(success(run(&client, &resume_args)).is_empty());
    assert!(agent.stop().success());
}

#[test]
fn log_cli_works_with_certificates() {
    let root = tempfile::tempdir().unwrap();
    let tls = root.path().join("tls");
    let setup = Command::new("sh").arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/dev-certs.sh")).arg(&tls).output().unwrap();
    success(setup);
    let ca = tls.join("ca.crt");
    let cert = tls.join("server.crt");
    let key = tls.join("server.key");
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap().to_string();
    drop(reservation);
    let log = root.path().join("agent.log");
    let output = fs::File::create(&log).unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/logs.json");
    let mut agent = Agent {
        child: command(root.path()).args(["agent", "--listen", &address, "--manifest", manifest.to_str().unwrap(), "--ca-cert", ca.to_str().unwrap(), "--cert", cert.to_str().unwrap(), "--private-key", key.to_str().unwrap()])
            .stdout(output.try_clone().unwrap()).stderr(output).spawn().unwrap(),
        log,
    };
    agent.ready();
    let client_cert = tls.join("client.crt");
    let client_key = tls.join("client.key");
    let args = ["logs", "--address", &address, "--app", "log-demo", "--ca-cert", ca.to_str().unwrap(), "--cert", client_cert.to_str().unwrap(), "--private-key", client_key.to_str().unwrap()];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = success(run(root.path(), &args));
        if output.contains("stdout] tick ") {
            break;
        }
        assert!(Instant::now() < deadline, "certificate logs did not arrive");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(agent.stop().success());
}

#[test]
fn local_device_agent_checks_keys_and_tokens_and_shuts_down() {
    let root = tempfile::tempdir().unwrap();
    let (server, server_public) = device(root.path(), "server");
    let (client, client_public) = device(root.path(), "client");
    let (stranger, stranger_public) = device(root.path(), "stranger");
    assert_ne!(
        fs::read(&server_public).unwrap(),
        fs::read(&client_public).unwrap()
    );
    approve(&server, "client", &client_public);
    approve(&client, "server", &server_public);
    approve(&client, "wrong-server", &stranger_public);
    approve(&stranger, "server", &server_public);

    let files: Vec<_> = [&server, &client]
        .into_iter()
        .flat_map(|home| {
            ["device-key.pem", "trusted-peers.json"].map(|name| {
                let path = home.join(".config/fleet/identity").join(name);
                let before = fs::read(&path).unwrap();
                (path, before)
            })
        })
        .collect();

    // keep test apps short in case the test stops
    let manifest = root.path().join("workload.json");
    fs::write(
        &manifest,
        r#"{"name":"network-test","program":"/bin/sleep","args":["2"],"replicas":1}"#,
    )
    .unwrap();
    let (mut agent, address) = Agent::start(&server, &manifest, root.path().join("agent.log"));

    let args = ["status-device", "--address", &address, "--peer", "server"];
    let status = success(run(&client, &args));
    assert!(status.contains("Workload: network-test\n"));
    assert!(status.contains("Desired copies: 1\n"));
    assert!(status.contains("Running copies: 1\n"));

    let bad_token = command(&client)
        .env("FLEET_TOKEN", "wrong-test-token")
        .args(args)
        .output()
        .unwrap();
    assert!(!bad_token.status.success());
    assert!(String::from_utf8_lossy(&bad_token.stderr).contains("Authentication failed"));
    assert!(bad_token.stdout.is_empty());

    let missing_token = command(&client)
        .env_remove("FLEET_TOKEN")
        .args(args)
        .output()
        .unwrap();
    assert!(!missing_token.status.success());
    assert!(String::from_utf8_lossy(&missing_token.stderr).contains("Your FLEET_TOKEN is not set"));
    assert!(missing_token.stdout.is_empty());

    let unknown_client = run(&stranger, &args);
    assert!(!unknown_client.status.success());
    assert!(unknown_client.stdout.is_empty());
    assert!(
        fs::read_to_string(&agent.log)
            .unwrap()
            .contains("Client key is not approved")
    );

    let wrong_server = run(
        &client,
        &[
            "status-device",
            "--address",
            &address,
            "--peer",
            "wrong-server",
        ],
    );
    assert!(!wrong_server.status.success());
    assert!(
        String::from_utf8_lossy(&wrong_server.stderr)
            .contains("Server key does not match the approved device")
    );
    assert!(wrong_server.stdout.is_empty());

    // bad requests must not stop good ones
    assert!(success(run(&client, &args)).contains("Running copies: 1\n"));

    let scale = [
        "scale-device",
        "--address",
        &address,
        "--peer",
        "server",
        "--replicas",
        "2",
    ];
    assert!(success(run(&client, &scale)).contains("Desired copies: 2\n"));
    wait_for_counts(&client, &address, 2);

    let invalid = run(
        &client,
        &[
            "scale-device",
            "--address",
            &address,
            "--peer",
            "server",
            "--replicas",
            "33",
        ],
    );
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("Replica count cannot exceed 32"));
    wait_for_counts(&client, &address, 2);

    let bad_scale = command(&client)
        .env("FLEET_TOKEN", "wrong-test-token")
        .args(scale)
        .output()
        .unwrap();
    assert!(!bad_scale.status.success());
    assert!(String::from_utf8_lossy(&bad_scale.stderr).contains("Authentication failed"));

    let stop = ["stop-device", "--address", &address, "--peer", "server"];
    let bad_stop = command(&client)
        .env("FLEET_TOKEN", "wrong-test-token")
        .args(stop)
        .output()
        .unwrap();
    assert!(!bad_stop.status.success());
    assert!(String::from_utf8_lossy(&bad_stop.stderr).contains("Authentication failed"));
    wait_for_counts(&client, &address, 2);

    let stopped = success(run(&client, &stop));
    assert!(stopped.contains("Desired copies: 0\n"));
    assert!(stopped.contains("Running copies: 0\n"));
    std::thread::sleep(Duration::from_millis(200));
    wait_for_counts(&client, &address, 0);

    success(run(
        &client,
        &[
            "scale-device",
            "--address",
            &address,
            "--peer",
            "server",
            "--replicas",
            "1",
        ],
    ));
    wait_for_counts(&client, &address, 1);
    assert!(agent.stop().success(), "agent should shut down cleanly");
    for line in fs::read_to_string(&agent.log).unwrap().lines() {
        if let Some(pid) = line.strip_prefix("Started process: ") {
            let pid = Pid::from_raw(pid.parse().unwrap()).unwrap();
            assert_eq!(
                test_kill_process(pid),
                Err(rustix::io::Errno::SRCH),
                "workload child must be reaped"
            );
        }
    }
    for (path, before) in files {
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn two_app_example_restarts_each_app_and_rejects_unnamed_commands() {
    let root = tempfile::tempdir().unwrap();
    let (server, server_public) = device(root.path(), "server");
    let (client, client_public) = device(root.path(), "client");
    approve(&server, "client", &client_public);
    approve(&client, "server", &server_public);
    let manifest = root.path().join("apps.json");
    fs::write(&manifest, include_str!("../examples/apps.json")).unwrap();
    let (mut agent, address) = Agent::start(&server, &manifest, root.path().join("agent.log"));
    let initial = agent.wait_for_apps(&[("demo-fast", 1), ("demo-slow", 2)], Duration::from_secs(5));
    assert_eq!(initial.len(), 3);

    for args in [
        vec!["status-device", "--address", &address, "--peer", "server"],
        vec!["stop-device", "--address", &address, "--peer", "server"],
        vec!["scale-device", "--address", &address, "--peer", "server", "--replicas", "0"],
    ] {
        let output = run(&client, &args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Choose an app before sending commands"));
    }
    let bad_token = command(&client).env("FLEET_TOKEN", "wrong-test-token")
        .args(["stop-device", "--address", &address, "--peer", "server"]).output().unwrap();
    assert!(!bad_token.status.success());
    assert!(String::from_utf8_lossy(&bad_token.stderr).contains("Authentication failed"));

    // the fast app must not restart the slow app
    agent.wait_for_apps(&[("demo-fast", 2), ("demo-slow", 2)], Duration::from_secs(15));
    for (name, pid) in &initial {
        if name == "demo-slow" {
            assert!(test_kill_process(*pid).is_ok(), "slow app restarted too soon");
        } else {
            assert_eq!(test_kill_process(*pid), Err(rustix::io::Errno::SRCH));
        }
    }

    agent.wait_for_apps(&[("demo-fast", 2), ("demo-slow", 4)], Duration::from_secs(35));
    assert!(agent.stop().success());
    for line in fs::read_to_string(&agent.log).unwrap().lines() {
        if let Some(pid) = line.strip_prefix("Started process: ") {
            let pid = Pid::from_raw(pid.parse().unwrap()).unwrap();
            assert_eq!(test_kill_process(pid), Err(rustix::io::Errno::SRCH), "child must be reaped");
        }
    }
}

#[test]
#[ignore = "Requires expect and a working pseudoterminal"]
fn device_dashboard_opens_logs_for_the_selected_app_and_returns_to_apps() {
    let root = tempfile::tempdir().unwrap();
    let (server, server_public) = device(root.path(), "server");
    let (controller, controller_public) = device(root.path(), "controller");
    approve(&server, "controller", &controller_public);
    approve(&controller, "local-one", &server_public);
    let manifest = root.path().join("apps.json");
    fs::write(&manifest, serde_json::to_vec(&serde_json::json!({ "workloads": [
        { "name": "alpha", "program": "/bin/sh", "args": ["-c", "printf 'alpha-output\\n'; sleep 60"], "replicas": 1 },
        { "name": "beta", "program": "/bin/sh", "args": ["-c", "printf 'beta-output\\n' >&2; sleep 60"], "replicas": 1 }
    ] })).unwrap()).unwrap();
    let log = root.path().join("agent.log");
    let (mut agent, address) = Agent::start(&server, &manifest, log.clone());
    let cluster = root.path().join("cluster.json");
    fs::write(&cluster, serde_json::to_vec(&serde_json::json!({ "nodes": [
        { "name": "local-one", "address": address, "server_name": "localhost", "peer": "local-one" }
    ] })).unwrap()).unwrap();
    let output = Command::new("expect")
        .env("HOME", &controller).env("FLEET_TOKEN", TOKEN).env("TERM", "xterm-256color")
        .env("FLEET_BIN", env!("CARGO_BIN_EXE_fleet")).env("FLEET_MANIFEST", &cluster)
        .args(["-c", r#"
set timeout 15
set stty_init "rows 24 columns 110"
log_user 0
proc see {text} {
    global spawn_id
    expect {
        -exact $text {}
        timeout {error "Did not see: $text"}
        eof {error "Dashboard exited before: $text"}
    }
}
if {[catch {
    spawn -noecho $env(FLEET_BIN) dashboard-device --manifest $env(FLEET_MANIFEST)
    see "beta"
    send -- "\tl"
    see "alpha-output"
    send -- "\033"
    after 300
    send -- "\033\[Bl"
    see "beta-output"
    send -- "+"
    after 300
    send -- "\033"
    after 300
    send -- "q"
    expect { eof {} timeout {error "Dashboard did not quit"} }
    set result [wait]
    if {[lindex $result 2] != 0 || [lindex $result 3] != 0} {error "Dashboard failed: $result"}
} message]} {
    puts stderr $message
    catch {close}
    catch {wait}
    exit 1
}
"#]).output().expect("run expect");
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(agent.child.try_wait().unwrap().is_none());
    let launches = agent.wait_for_apps(&[("alpha", 1), ("beta", 1)], Duration::from_secs(5));
    assert_eq!(launches.len(), 2, "log keys must not scale an app");
    assert!(agent.stop().success());
}

#[test]
#[ignore = "Requires expect and a working pseudoterminal"]
fn device_dashboard_scales_the_selected_app_only() {
    let root = tempfile::tempdir().unwrap();
    let (server, server_public) = device(root.path(), "server");
    let (controller, controller_public) = device(root.path(), "controller");
    approve(&server, "controller", &controller_public);
    approve(&controller, "local-one", &server_public);
    let manifest = root.path().join("apps.json");
    fs::write(&manifest, serde_json::to_vec(&serde_json::json!({ "workloads": [
        { "name": "demo-fast", "program": "/bin/sleep", "args": ["60"], "replicas": 1 },
        { "name": "demo-slow", "program": "/bin/sleep", "args": ["60"], "replicas": 2 }
    ] })).unwrap()).unwrap();
    let log = root.path().join("agent.log");
    let (mut agent, address) = Agent::start(&server, &manifest, log.clone());
    let cluster = root.path().join("cluster.json");
    fs::write(&cluster, serde_json::to_vec(&serde_json::json!({ "nodes": [
        { "name": "local-one", "address": address, "server_name": "localhost", "peer": "local-one" }
    ] })).unwrap()).unwrap();
    let output = Command::new("expect")
        .env("HOME", &controller).env("FLEET_TOKEN", TOKEN).env("TERM", "xterm-256color")
        .env("FLEET_BIN", env!("CARGO_BIN_EXE_fleet"))
        .env("FLEET_MANIFEST", &cluster).env("FLEET_AGENT_LOG", &log)
        .args(["-c", r#"
set timeout 15
set stty_init "rows 24 columns 110"
log_user 0
proc wait_log {text count} {
    global env spawn_id
    for {set attempt 0} {$attempt < 50} {incr attempt} {
        expect -timeout 0 {
            -re {.+} {exp_continue}
            timeout {}
            eof {error "Dashboard exited early"}
        }
        set file [open $env(FLEET_AGENT_LOG) r]
        set log [read $file]
        close $file
        if {[regexp -all -- $text $log] >= $count} {return}
        after 100
    }
    error "Agent did not log: $text"
}
if {[catch {
    spawn -noecho $env(FLEET_BIN) dashboard-device --manifest $env(FLEET_MANIFEST)
    expect {
        -exact "demo-slow" {}
        timeout {error "App table did not load"}
        eof {error "Dashboard exited early"}
    }
    send -- "\t+\033\[B"
    wait_log "Starting workload: demo-fast" 2
    after 200
    send -- "-"
    wait_log "Stopping one copy of demo-slow" 1
    send -- "q"
    expect { eof {} timeout {error "Dashboard did not quit"} }
    set result [wait]
    if {[lindex $result 2] != 0 || [lindex $result 3] != 0} {error "Dashboard failed: $result"}
} message]} {
    puts stderr $message
    catch {close}
    catch {wait}
    exit 1
}
"#]).output().expect("run expect");
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let launches = agent.wait_for_apps(&[("demo-fast", 2), ("demo-slow", 2)], Duration::from_secs(5));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let live = |name: &str| launches.iter().filter(|(app, pid)| app == name && test_kill_process(*pid).is_ok()).count();
        if live("demo-fast") == 2 && live("demo-slow") == 1 { break; }
        assert!(Instant::now() < deadline, "wrong app counts after dashboard scaling");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(agent.stop().success());
}

#[test]
#[ignore = "Requires expect and a working pseudoterminal"]
fn device_dashboard_and_watch_use_the_right_key_for_each_agent() {
    let root = tempfile::tempdir().unwrap();
    let (first, first_public) = device(root.path(), "first");
    let (second, second_public) = device(root.path(), "second");
    let (controller, controller_public) = device(root.path(), "controller");
    assert_ne!(
        fs::read(&first_public).unwrap(),
        fs::read(&second_public).unwrap()
    );
    approve(&first, "controller", &controller_public);
    approve(&second, "controller", &controller_public);
    approve(&controller, "local-one", &first_public);
    approve(&controller, "local-two", &second_public);

    let first_manifest = root.path().join("first.json");
    let second_manifest = root.path().join("second.json");
    for (path, replicas) in [(&first_manifest, 1), (&second_manifest, 3)] {
        fs::write(path, serde_json::to_vec(&serde_json::json!({
            "name": "dashboard-test", "program": "/bin/sleep", "args": ["10"], "replicas": replicas
        })).unwrap()).unwrap();
    }
    let (mut first_agent, first_address) =
        Agent::start(&first, &first_manifest, root.path().join("first.log"));
    let (mut second_agent, second_address) =
        Agent::start(&second, &second_manifest, root.path().join("second.log"));
    let cluster = root.path().join("cluster.json");
    let bad_cluster = root.path().join("wrong-keys.json");
    let mut nodes = serde_json::json!({ "nodes": [
        { "name": "local-one", "address": first_address, "server_name": "localhost", "peer": "local-one" },
        { "name": "local-two", "address": second_address, "server_name": "localhost", "peer": "local-two" }
    ]});
    fs::write(&cluster, serde_json::to_vec(&nodes).unwrap()).unwrap();
    nodes["nodes"][0]["peer"] = "local-two".into();
    nodes["nodes"][1]["peer"] = "local-one".into();
    fs::write(&bad_cluster, serde_json::to_vec(&nodes).unwrap()).unwrap();

    let output = Command::new("expect")
        .env("HOME", &controller).env("FLEET_TOKEN", TOKEN).env("TERM", "xterm-256color")
        .env("FLEET_BIN", env!("CARGO_BIN_EXE_fleet"))
        .env("FLEET_MANIFEST", &cluster).env("FLEET_BAD_MANIFEST", &bad_cluster)
        .env("FLEET_FIRST", &first_address).env("FLEET_SECOND", &second_address)
        .args(["-c", r#"
set timeout 15
set stty_init "rows 30 columns 110"
log_user 0
proc see {text} {
    global spawn_id
    expect {
        -exact $text {}
        timeout {error "Timed out waiting for: $text"}
        eof {error "Process exited before showing: $text"}
    }
}
proc counts {address peer count {drain 1}} {
    global env spawn_id
    for {set attempt 0} {$attempt < 40} {incr attempt} {
        if {$drain} {
            expect -timeout 0 {
                -re {.+} {exp_continue}
                timeout {}
                eof {error "Dashboard exited early"}
            }
        }
        set status [exec $env(FLEET_BIN) status-device --address $address --peer $peer]
        if {[string first "Desired copies: $count\n" $status] >= 0 &&
            [string first "Running copies: $count" $status] >= 0} {return}
        after 100
    }
    error "$peer did not reach $count copies: $status"
}
proc quit_dashboard {} {
    global spawn_id
    send -- "q"
    expect {
        eof {}
        timeout {error "Dashboard did not quit"}
    }
    set result [wait]
    if {[lindex $result 2] != 0 || [lindex $result 3] != 0} {error "Dashboard failed: $result"}
}
if {[catch {
    spawn -noecho $env(FLEET_BIN) dashboard-device --manifest $env(FLEET_MANIFEST)
    see "dashboard-test"
    send -- "\t+"
    counts $env(FLEET_FIRST) local-one 2
    counts $env(FLEET_SECOND) local-two 3
    after 250
    send -- "\t\033\[B\t-"
    counts $env(FLEET_SECOND) local-two 2
    counts $env(FLEET_FIRST) local-one 2
    quit_dashboard

    spawn -noecho $env(FLEET_BIN) dashboard-device --manifest $env(FLEET_BAD_MANIFEST)
    see "Unreachable"
    see "Unreachable"
    quit_dashboard
    counts $env(FLEET_FIRST) local-one 2 0
    counts $env(FLEET_SECOND) local-two 2 0

    spawn -noecho $env(FLEET_BIN) watch-device --manifest $env(FLEET_MANIFEST)
    set seen_one 0
    set seen_two 0
    while {!$seen_one || !$seen_two} {
        expect {
            -exact "local-one: running 2, desired 2" {set seen_one 1}
            -exact "local-two: running 2, desired 2" {set seen_two 1}
            timeout {error "Watch did not report both agents"}
            eof {error "Watch exited early"}
        }
    }
    send -- "\003"
    expect {
        eof {}
        timeout {error "Watch did not quit"}
    }
    wait
    counts $env(FLEET_FIRST) local-one 2 0
    counts $env(FLEET_SECOND) local-two 2 0
} message]} {
    puts stderr $message
    catch {close}
    catch {wait}
    exit 1
}
puts "Two device-key agents: dashboard controls, wrong-key rejection, watch, and clean UI exit passed."
exit 0
"#]).output().expect("run expect; install it to run this opt-in test");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(first_agent.stop().success());
    assert!(second_agent.stop().success());
}
