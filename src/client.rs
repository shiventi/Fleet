//! Sends JSON commands over mutual TLS and prints the agent's replies.
//!
//! Each client loads `FLEET_TOKEN` and TLS files once. The token must match the agent's.
//! Both sides verify certificates before any command or token is sent.

use crate::protocol::{AuthenticatedRequest, Request, Response, StatusResponse};
use crate::tls::{self, TlsFiles};
use rustls::ClientConfig;
use std::io::{self, Write};
use std::sync::Arc;

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

/// Holds a token and TLS config for repeated requests, including across threads.
/// Each request opens a new connection. Create a new client to reload credentials.
pub struct FleetClient {
    token: String,
    config: Arc<ClientConfig>,
}

impl FleetClient {
    /// Loads the token and TLS files without connecting to an agent.
    ///
    /// # Errors
    /// Returns an error if the token is missing or blank, or the TLS files cannot be loaded.
    pub fn new(files: &TlsFiles) -> io::Result<Self> {
        let token = std::env::var("FLEET_TOKEN")
            .map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

        if token.trim().is_empty() {
            return Err(io::Error::other("Your FLEET_TOKEN is blank"));
        }

        let config = Arc::new(tls::client_config(files)?);

        Ok(Self { token, config })
    }
    /// Gets the agent's status without printing it.
    ///
    /// # Errors
    /// Returns an error if connecting, sending, reading, or parsing fails,
    /// or the agent rejects the request.
    pub fn fetch_status(&self, address: &str, server_name: &str) -> io::Result<StatusResponse> {
        let mut stream = tls::connect(address, server_name, Arc::clone(&self.config))?;
        let request = AuthenticatedRequest {
            token: self.token.clone(),
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

    /// Asks the agent to stop its workload and prints the updated counts.
    ///
    /// The agent stays running and the manifest file is unchanged.
    ///
    /// # Errors
    /// Returns an error if connecting, sending, reading, or parsing fails,
    /// or the agent rejects the request.
    pub fn stop(&self, address: &str, server_name: &str) -> io::Result<()> {
        let mut stream = tls::connect(address, server_name, Arc::clone(&self.config))?;
        let request = AuthenticatedRequest {
            token: self.token.clone(),
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
    /// Returns an error if connecting, sending, reading, or parsing fails,
    /// or the agent rejects the request.
    pub fn scale(&self, address: &str, replicas: u32, server_name: &str) -> io::Result<()> {
        let mut stream = tls::connect(address, server_name, Arc::clone(&self.config))?;
        let request = AuthenticatedRequest {
            token: self.token.clone(),
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
}

/// Loads a client and gets one agent's status without printing it.
///
/// # Errors
/// Returns an error if client setup or the status request fails.
pub fn fetch_status(
    address: &str,
    server_name: &str,
    files: &TlsFiles,
) -> io::Result<StatusResponse> {
    let client = FleetClient::new(files)?;
    return client.fetch_status(address, server_name);
}

/// Loads a client, stops the workload, and prints the updated counts.
///
/// # Errors
/// Returns an error if client setup or the stop request fails.
pub fn stop(address: &str, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let client = FleetClient::new(files)?;
    return client.stop(address, server_name);
}

/// Loads a client, sets the desired copy count, and prints the current counts.
///
/// # Errors
/// Returns an error if client setup or the scale request fails.
pub fn scale(address: &str, replicas: u32, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let client = FleetClient::new(files)?;
    return client.scale(address, replicas, server_name);
}
