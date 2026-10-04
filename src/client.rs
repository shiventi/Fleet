//! Sends JSON commands to an agent and prints its replies.

use crate::protocol::{Request, StatusResponse};
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;

/// Connects to an agent and displays its workload and process counts.
///
/// # Errors
/// Returns an error if the connection fails or the reply cannot be read or parsed.
pub fn status(address: &str) -> io::Result<()> {
    let mut stream = TcpStream::connect(address)?;
    let request = Request::Status;
    let json = serde_json::to_string(&request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

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

/// Asks the agent to stop its workload and prints the updated counts.
///
/// The agent stays running and the manifest file is unchanged.
///
/// # Errors
/// Returns an error if the connection fails or the reply cannot be read or parsed.
pub fn stop(address: &str) -> io::Result<()> {
    let mut stream = TcpStream::connect(address)?;
    let request = Request::Stop;
    let json = serde_json::to_string(&request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    let mut response = String::new();
    let mut reader = BufReader::new(stream);

    reader.read_line(&mut response)?;
    let status: StatusResponse = serde_json::from_str(&response)?;

    println!("Desired copies: {}", status.desired);
    println!("Running copies: {}", status.running);

    Ok(())
}

/// Sets the desired copy count and prints the agent's current counts.
/// The running count may take another agent loop to reach the target.
///
/// # Errors
/// Returns an error if connecting, sending, reading, or parsing fails.
pub fn scale(address: &str, replicas: u32) -> io::Result<()> {
    let mut stream = TcpStream::connect(address)?;
    let request = Request::Scale { replicas };
    let json = serde_json::to_string(&request)?;

    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut response)?;

    let status: StatusResponse = serde_json::from_str(&response)?;

    println!("Desired copies: {}", status.desired);
    println!("Running copies: {}", status.running);

    Ok(())
}
