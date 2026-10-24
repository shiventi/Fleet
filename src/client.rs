use crate::device_tls;
use crate::identity::DeviceIdentity;
use crate::protocol::{AuthenticatedRequest, Request, Response, StatusResponse};
use crate::runner::LogEntry;
use crate::tls::{self, TlsFiles};
use crate::trust::TrustedPeer;
use rustls::ClientConfig;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

// share tls setup across calls
#[derive(Clone)]
pub struct FleetClient {
    token: String,
    config: Arc<ClientConfig>,
}

impl FleetClient {
    pub fn new(files: &TlsFiles) -> io::Result<Self> {
        Self::from_config(tls::client_config(files)?)
    }
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
            Response::Apps { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received app list")),
            Response::Logs { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received logs")),
        }
    }

    pub fn list_apps(&self, address: &str, server_name: &str) -> io::Result<Vec<StatusResponse>> {
        let mut stream = tls::connect(address, server_name, Arc::clone(&self.config))?;
        let request = AuthenticatedRequest {
            token: self.token.clone(),
            request: Request::ListApps,
        };
        let json = serde_json::to_string(&request)?;
        stream.write_all(json.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let response = tls::read_message(&mut stream)?;
        let reply: Response = serde_json::from_str(&response)?;
        match reply {
            Response::Apps { apps } => Ok(apps),
            Response::Error { message } => Err(io::Error::other(message)),
            Response::Status { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app list, received app status")),
            Response::Logs { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app list, received logs")),
        }
    }

    pub fn fetch_logs(&self, address: &str, workload_name: &str, after_id: u64, server_name: &str) -> io::Result<Vec<LogEntry>> {
        let mut stream = tls::connect(address, server_name, Arc::clone(&self.config))?;
        let request = AuthenticatedRequest {
            token: self.token.clone(),
            request: Request::LogsApp { workload_name: workload_name.to_owned(), after_id },
        };
        let json = serde_json::to_string(&request)?;
        stream.write_all(json.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let response = tls::read_message(&mut stream)?;
        let reply: Response = serde_json::from_str(&response)?;
        match reply {
            Response::Logs { entries } => Ok(entries),
            Response::Error { message } => Err(io::Error::other(message)),
            Response::Status { .. } | Response::Apps { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected logs, received app status or app list")),
        }
    }

    pub fn logs(&self, address: &str, workload_name: &str, mut after_id: u64, server_name: &str, follow: bool) -> io::Result<()> {
        let keep_alive = Arc::new(AtomicBool::new(true));
        if follow {
            let flag = Arc::clone(&keep_alive);
            ctrlc::set_handler(move || flag.store(false, Ordering::SeqCst)).map_err(io::Error::other)?;
        }
        while keep_alive.load(Ordering::SeqCst) {
            let entries = match self.fetch_logs(address, workload_name, after_id, server_name) {
                Ok(entries) => entries,
                Err(_) if !keep_alive.load(Ordering::SeqCst) => break,
                Err(error) => return Err(error),
            };
            if let Err(error) = print_logs(&mut io::stdout().lock(), &entries, &mut after_id) {
                if error.kind() == io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(error);
            }
            if !follow {
                break;
            }
            if entries.is_empty() {
                for _ in 0..10 {
                    if !keep_alive.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        Ok(())
    }

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
            Response::Apps { .. } => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received app list"));
            }
            Response::Logs { .. } => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received logs"));
            }
        }

        Ok(())
    }

    pub fn scale(&self, address: &str, replicas: u32, server_name: &str) -> io::Result<StatusResponse> {
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
            Response::Status { status } => Ok(status),
            Response::Error { message } => Err(io::Error::other(message)),
            Response::Apps { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received app list")),
            Response::Logs { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received logs")),
        }
    }

    pub fn scale_app(&self, address: &str, workload_name: &str, replicas: u32, server_name: &str) -> io::Result<StatusResponse> {
        let mut stream = tls::connect(address, server_name, Arc::clone(&self.config))?;
        let request = AuthenticatedRequest {
            token: self.token.clone(),
            request: Request::ScaleApp { workload_name: workload_name.to_owned(), replicas },
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
            Response::Apps { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received app list")),
            Response::Logs { .. } => Err(io::Error::new(io::ErrorKind::InvalidData, "Expected app status, received logs")),
        }
    }

    fn from_config(config: ClientConfig) -> io::Result<Self> {
        let token = std::env::var("FLEET_TOKEN").map_err(|_| io::Error::other("Your FLEET_TOKEN is not set"))?;

        if token.trim().is_empty() {
            return Err(io::Error::other("Your FLEET_TOKEN is blank"));
        }

        Ok(Self { token, config: Arc::new(config) })
    }

    pub fn new_device(identity: &DeviceIdentity, peer: &TrustedPeer) -> io::Result<Self> {
        Self::from_config(device_tls::client_config(identity, peer)?)
    }

    pub fn for_peer(peer_name: &str) -> io::Result<Self> {
        let identity = DeviceIdentity::load_default()?;
        let directory = crate::identity::default_directory()?;
        let trust = crate::trust::TrustStore::load(&directory.join("trusted-peers.json"))?;

        let saved_peer = match trust.peers().iter().find(|saved| saved.name == peer_name) {
            Some(saved) => saved,
            None => {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("Unknown peer: {}", peer_name),))
            }
        };
        Self::new_device(&identity, saved_peer)
    }
}

pub(crate) fn print_logs(writer: &mut impl Write, entries: &[LogEntry], after_id: &mut u64) -> io::Result<()> {
    let mut previous = *after_id;
    for entry in entries {
        if entry.id <= previous {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Log IDs must increase"));
        }
        previous = entry.id;
    }
    for entry in entries {
        let stream = match entry.stream {
            crate::runner::LogStream::Stdout => "stdout",
            crate::runner::LogStream::Stderr => "stderr",
        };
        let text = log_text(&entry.text);
        writeln!(writer, "[{} pid={} {}] {}", entry.id, entry.pid, stream, text)?;
        *after_id = entry.id;
    }
    writer.flush()
}

pub fn log_text(text: &str) -> String {
    // show control chars as text, not terminal commands
    let mut safe = String::new();
    for character in text.chars() {
        if character.is_control() {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
    }
    safe
}

pub fn fetch_status(address: &str, server_name: &str, files: &TlsFiles) -> io::Result<StatusResponse> {
    let client = FleetClient::new(files)?;
    return client.fetch_status(address, server_name);
}

pub fn stop(address: &str, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let client = FleetClient::new(files)?;
    return client.stop(address, server_name);
}

pub fn scale(address: &str, replicas: u32, server_name: &str, files: &TlsFiles) -> io::Result<()> {
    let client = FleetClient::new(files)?;
    let status = client.scale(address, replicas, server_name)?;
    println!("Desired copies: {}", status.desired);
    println!("Running copies: {}", status.running);
    Ok(())
}

#[cfg(test)]
mod tests;
