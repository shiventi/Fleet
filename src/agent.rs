//! Manages workload copies and handles commands over mutual TLS.

use crate::protocol::{AuthenticatedRequest, Request, Response, StatusResponse};
use crate::runner::ProcessRunner;
use crate::tls::{self, ServerStream, TlsFiles};
use crate::workload::{MAX_REPLICAS, WorkloadSpec};
use crate::{Action, ObservedWorkload, plan};
use std::io::{self, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use subtle::ConstantTimeEq;

/// Adds or removes copies to match the target and handles one client at a time.
///
/// Ctrl+C ends the loop and stops direct child processes.
/// Client I/O can delay checks for up to five seconds per connection.
/// Reads `FLEET_TOKEN` once at startup and checks it before handling any command.
/// Requires a trusted client certificate. Remote addresses need `allow_remote`.
/// Wildcard addresses such as `0.0.0.0` and `::` are rejected.
///
/// # Errors
/// Returns an error if `FLEET_TOKEN` is missing, blank, or not valid text,
/// TLS files are invalid, or the listener or Ctrl+C handler cannot be set up.
/// Later connection errors are logged and the loop continues.
///
/// # Panics
/// Panics if a process cannot be started, checked, or stopped, or its count exceeds u32.
pub fn allow_connection(
    address: &str,
    mut desired: WorkloadSpec,
    files: &TlsFiles,
    allow_remote: bool,
) -> io::Result<()> {
    let expected_token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if expected_token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }

    let listen_address: std::net::SocketAddr = address.parse().map_err(io::Error::other)?;

    if listen_address.ip().is_unspecified() {
        return Err(io::Error::other(
            "Please use a specific IP address, not 0.0.0.0 or ::",
        ));
    }

    if !listen_address.ip().is_loopback() && !allow_remote {
        return Err(io::Error::other("Remote addresses require --allow-remote"));
    }

    let config = Arc::new(tls::server_config(files)?);
    let mut runner = ProcessRunner::new();
    let listener = TcpListener::bind(listen_address)?;
    listener.set_nonblocking(true)?;
    let keep_alive = Arc::new(AtomicBool::new(true));
    let handler_keep_alive = Arc::clone(&keep_alive);
    ctrlc::set_handler(move || handler_keep_alive.store(false, Ordering::SeqCst))
        .map_err(io::Error::other)?;
    println!("Listening on: {}", address);
    println!("Agent workload: {:?}", desired);

    while keep_alive.load(Ordering::SeqCst) {
        let count = runner.running_count();
        let observed = ObservedWorkload {
            name: desired.name.clone(),
            running: u32::try_from(count).expect("process count exceeds u32"),
        };
        let actions = plan(&desired, &observed);
        for action in actions {
            if !keep_alive.load(Ordering::SeqCst) {
                break;
            }
            match action {
                Action::Start { workload_name } => {
                    println!("Starting workload: {}", workload_name);
                    runner.start(&desired);
                }
                Action::Stop { workload_name } => {
                    println!("Stopping one copy of {}", workload_name);
                    runner.stop_one();
                }
            }
        }
        match listener.accept() {
            Ok((stream, peer_address)) => {
                println!("Connected: {}", peer_address);
                let result = tls::accept(stream, Arc::clone(&config)).and_then(|mut stream| {
                    let result =
                        handle_client(&mut stream, &mut desired, &mut runner, &expected_token);
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                    result
                });
                if let Err(e) = result {
                    eprintln!("error handling client {}: {}", peer_address, e);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // No client is waiting; this is normal in nonblocking mode.
            }
            Err(e) => {
                eprintln!("connection failed: {}", e);
            }
        }
        // Avoid checking continuously and wasting CPU.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    runner.stop_all();
    Ok(())
}

/// Sends one JSON reply followed by a newline.
///
/// # Errors
/// Returns an error if encoding or writing fails.
fn send_reply(writer: &mut ServerStream, reply: &Response) -> io::Result<()> {
    let json = serde_json::to_string(reply)?;
    writer.write_all(json.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Checks the request's token before handling its command.
///
/// # Errors
/// Returns an error if reading, writing, or encoding fails.
/// Invalid requests or wrong tokens receive an error reply without changing the workload.
fn handle_client(
    stream: &mut ServerStream,
    desired: &mut WorkloadSpec,
    runner: &mut ProcessRunner,
    expected_token: &str,
) -> io::Result<()> {
    let input = tls::read_message(stream)?;
    let authenticated = match serde_json::from_str::<AuthenticatedRequest>(&input) {
        Ok(request) => request,
        Err(_) => {
            let reply = Response::Error {
                message: String::from("Invalid request"),
            };
            return send_reply(stream, &reply);
        }
    };

    if authenticated
        .token
        .as_bytes()
        .ct_eq(expected_token.as_bytes())
        .unwrap_u8()
        == 0
    {
        let reply = Response::Error {
            message: String::from("Authentication failed"),
        };
        return send_reply(stream, &reply);
    }

    match authenticated.request {
        Request::Status => {
            let response = StatusResponse {
                workload_name: Some(desired.name.clone()),
                desired: desired.replicas,
                running: runner.running_count(),
            };

            let reply = Response::Status { status: response };
            send_reply(stream, &reply)?;
        }
        Request::Stop => {
            // Change the target first so the next loop does not restart the workload.
            desired.replicas = 0;
            runner.stop_all();

            let response = StatusResponse {
                workload_name: Some(desired.name.clone()),
                desired: desired.replicas,
                running: runner.running_count(),
            };

            let reply = Response::Status { status: response };
            send_reply(stream, &reply)?;
        }
        Request::Scale { replicas } => {
            // Reject the request before changing the agent's target.
            if replicas > MAX_REPLICAS {
                let reply = Response::Error {
                    message: format!("Replica count cannot exceed {}", MAX_REPLICAS),
                };
                return send_reply(stream, &reply);
            }
            // The next supervisor loop brings the running count to this target.
            desired.replicas = replicas;
            let response = StatusResponse {
                workload_name: Some(desired.name.clone()),
                desired: desired.replicas,
                running: runner.running_count(),
            };

            let reply = Response::Status { status: response };
            send_reply(stream, &reply)?;
        }
    }

    Ok(())
}
