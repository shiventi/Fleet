//! Sends token-authenticated JSON commands to an agent and prints its replies.
//!
//! Reads `FLEET_TOKEN` before connecting. It must match the agent's token.
//! Use localhost or an encrypted tunnel; TCP sends the token without encryption.

use crate::protocol::{AuthenticatedRequest, Request, Response};
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;

/// Connects to an agent and displays its workload and process counts.
///
/// # Errors
/// Returns an error if the token is missing or invalid, connecting, sending,
/// reading, or parsing fails, or the agent rejects the request.
pub fn status(address: &str) -> io::Result<()> {
    let token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }
    let mut stream = TcpStream::connect(address)?;
    let request = AuthenticatedRequest {
        token,
        request: Request::Status,
    };
    let json = serde_json::to_string(&request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    let mut response = String::new();
    let mut reader = BufReader::new(stream);

    reader.read_line(&mut response)?;
    let reply: Response = serde_json::from_str(&response)?;

    match reply {
        Response::Status { status } => {
            match status.workload_name {
                Some(name) => println!("Workload: {}", name),
                None => println!("Workload: none"),
            }
            println!("Desired copies: {}", status.desired);
            println!("Running copies: {}", status.running);
        }
        Response::Error { message } => {
            return Err(io::Error::other(message));
        }
    }

    Ok(())
}

/// Asks the agent to stop its workload and prints the updated counts.
///
/// The agent stays running and the manifest file is unchanged.
///
/// # Errors
/// Returns an error if the token is missing or invalid, connecting, sending,
/// reading, or parsing fails, or the agent rejects the request.
pub fn stop(address: &str) -> io::Result<()> {
    let token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }
    let mut stream = TcpStream::connect(address)?;
    let request = AuthenticatedRequest {
        token,
        request: Request::Stop,
    };
    let json = serde_json::to_string(&request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    let mut response = String::new();
    let mut reader = BufReader::new(stream);

    reader.read_line(&mut response)?;
    let reply: Response = serde_json::from_str(&response)?;
    match reply {
        Response::Status { status } => {
            println!("Desired copies: {}", status.desired);
            println!("Running copies: {}", status.running);
        }
        Response::Error { message } => {
            return Err(io::Error::other(message));
        }
    }

    Ok(())
}

/// Sets the desired copy count and prints the agent's current counts.
/// The running count may take another agent loop to reach the target.
///
/// # Errors
/// Returns an error if the token is missing or invalid, connecting, sending,
/// reading, or parsing fails, or the agent rejects the request.
pub fn scale(address: &str, replicas: u32) -> io::Result<()> {
    let token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }
    let mut stream = TcpStream::connect(address)?;
    let request = AuthenticatedRequest {
        token,
        request: Request::Scale { replicas },
    };
    let json = serde_json::to_string(&request)?;

    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut response)?;

    let reply: Response = serde_json::from_str(&response)?;
    match reply {
        Response::Status { status } => {
            println!("Desired copies: {}", status.desired);
            println!("Running copies: {}", status.running);
        }
        Response::Error { message } => {
            return Err(io::Error::other(message));
        }
    }

    Ok(())
}
