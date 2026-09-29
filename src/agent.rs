//! Keeps a workload running and answers TCP status requests.

use crate::protocol::StatusResponse;
use crate::runner::ProcessRunner;
use crate::workload::WorkloadSpec;
use crate::{Action, ObservedWorkload, plan};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Restarts missing copies and handles one client at a time.
///
/// Ctrl+C ends the loop and stops direct child processes.
/// Client I/O can delay checks; idle reads and writes time out after one second.
///
/// # Errors
/// Returns an error if the listener or Ctrl+C handler cannot be set up.
/// Later connection errors are logged and the loop continues.
///
/// # Panics
/// Panics if a process cannot be started, checked, or stopped, or its count exceeds u32.
pub fn allow_connection(address: &str, desired: WorkloadSpec) -> io::Result<()> {
    let mut runner = ProcessRunner::new();
    let listener = TcpListener::bind(address)?;
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
                    println!("Stopping isn't implemented yet for {}", workload_name);
                }
            }
        }
        match listener.accept() {
            Ok((stream, peer_address)) => {
                println!("Connected: {}", peer_address);
                if let Err(e) = handle_client(stream, &desired, &mut runner) {
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

/// Reads one command and sends a JSON status or a plain-text error.
///
/// # Errors
/// Returns an error if reading, writing, or encoding the reply fails.
fn handle_client(
    stream: TcpStream,
    desired: &WorkloadSpec,
    runner: &mut ProcessRunner,
) -> io::Result<()> {
    // Use blocking reads with a timeout, regardless of the listener's mode.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;
    let mut input = String::new();

    reader.read_line(&mut input)?;

    match input.trim_end() {
        "status" => {
            let response = StatusResponse {
                workload_name: Some(desired.name.clone()),
                desired: desired.replicas,
                running: runner.running_count(),
            };

            let json = serde_json::to_string(&response)?;
            writer.write_all(json.as_bytes())?;
            // The newline marks the end of the reply for the client's read_line.
            writer.write_all(b"\n")?;
        }
        _ => {
            writer.write_all(b"error: unknown command\n")?;
        }
    }

    writer.flush()?;
    Ok(())
}
