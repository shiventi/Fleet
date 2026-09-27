//! Requests status from an agent and prints its reply.

use crate::protocol::StatusResponse;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;

/// Connects to an agent and displays its workload and process counts.
///
/// # Errors
/// Returns an error if the connection fails or the reply cannot be read or parsed.
pub fn status(address: &str) -> io::Result<()> {
    let mut stream = TcpStream::connect(address)?;
    stream.write_all(b"status\n")?;

    let mut response = String::new();
    let mut reader = BufReader::new(stream);

    reader.read_line(&mut response)?;
    let status: StatusResponse = serde_json::from_str(&response)?;
    match status.workload_name {
        Some(name) => println!("Workload: {}", name),
        None => println!("Workload: none"),
    }
    println!("Desired copies: {}", status.desired);
    println!("Running copies: {}", status.running);

    Ok(())
}
