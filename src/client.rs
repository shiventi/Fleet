//! Sends JSON commands over mutual TLS and prints the agent's replies.
//!
//! Reads `FLEET_TOKEN` before connecting. It must match the agent's token.
//! Both sides verify certificates before any command or token is sent.

use crate::protocol::{AuthenticatedRequest, Request, Response, StatusResponse};
use crate::tls::{self, TlsFiles};
use std::io::{self, Write};

/// Gets the agent's status without printing it.
///
/// # Errors
/// Returns an error if the token is missing or invalid, connecting, sending,
/// reading, or parsing fails, or the agent rejects the request.
pub fn fetch_status(
    address: &str,
    server_name: &str,
    files: &TlsFiles,
) -> io::Result<StatusResponse> {
    let token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }
    let mut stream = tls::connect(address, server_name, files)?;
    let request = AuthenticatedRequest {
        token,
        request: Request::Status,
    };
    let json = serde_json::to_string(&request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    stream.flush()?;
    let response = tls::read_message(&mut stream)?;
    let reply: Response = serde_json::from_str(&response)?;

    match reply {
        Response::Status { status } => Ok(status),
        Response::Error { message } => Err(io::Error::other(message)),
    }
}

/// Gets the agent's status and prints its name and copy counts.
///
/// # Errors
/// Returns an error if fetching the status fails.
pub fn status(address: &str, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let status = fetch_status(address, server_name, files)?;

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
/// Returns an error if the token is missing or invalid, connecting, sending,
/// reading, or parsing fails, or the agent rejects the request.
pub fn stop(address: &str, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }
    let mut stream = tls::connect(address, server_name, files)?;
    let request = AuthenticatedRequest {
        token,
        request: Request::Stop,
    };
    let json = serde_json::to_string(&request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    stream.flush()?;
    let response = tls::read_message(&mut stream)?;
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
pub fn scale(address: &str, replicas: u32, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let token = std::env::var("FLEET_TOKEN")
        .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

    if token.trim().is_empty() {
        return Err(io::Error::other("Your FLEET_TOKEN is blank"));
    }
    let mut stream = tls::connect(address, server_name, files)?;
    let request = AuthenticatedRequest {
        token,
        request: Request::Scale { replicas },
    };
    let json = serde_json::to_string(&request)?;

    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;

    stream.flush()?;
    let response = tls::read_message(&mut stream)?;

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
