//! Accepts TCP requests and replies with the agent's status.

use crate::protocol::StatusResponse;
use crate::workload::WorkloadSpec;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

/// Keeps listening and handles one client at a time.
///
/// The workload is only printed for now; no processes are started.
/// Client reads have no timeout yet and can delay the loop.
///
/// # Errors
/// Returns an error if binding or setting nonblocking mode fails.
/// Later connection errors are logged and the loop continues.
pub fn allow_connection(address: &str, desired: WorkloadSpec) -> io::Result<()> {
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    println!("Listening on: {}", address);
    println!("Agent workload: {:?}", desired);

    loop {
        match listener.accept() {
            Ok((_stream, peer_address)) => {
                println!("Connected: {}", peer_address);
                if let Err(e) = handle_client(_stream) {
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
}

/// Reads one command and sends a JSON status or a plain-text error.
///
/// # Errors
/// Returns an error if reading, writing, or encoding the reply fails.
fn handle_client(stream: TcpStream) -> io::Result<()> {
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;
    let mut input = String::new();

    reader.read_line(&mut input)?;

    match input.trim_end() {
        "status" => {
            // Report an empty agent until process management is connected.
            let response = StatusResponse {
                workload_name: None,
                desired: 0,
                running: 0,
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
