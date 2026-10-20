use crate::device_tls;
use crate::identity::DeviceIdentity;
use crate::protocol::{AuthenticatedRequest, Request, Response, StatusResponse};
use crate::runner::ProcessRunner;
use crate::tls::{self, ServerStream, TlsFiles};
use crate::trust::TrustStore;
use crate::workload::{MAX_REPLICAS, WorkloadSpec};
use crate::{Action, ObservedWorkload, plan};
use rustls::ServerConfig;
use std::io::{self, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

//keep app state seperate.
struct ManagedWorkload {
    spec: WorkloadSpec,
    runner: ProcessRunner,
    retry_at: Instant,
}

impl ManagedWorkload {
    fn new(spec: WorkloadSpec) -> Self {
        Self { spec, runner: ProcessRunner::new(), retry_at: Instant::now() }
    }

    fn tick(&mut self, keep_alive: &AtomicBool) {
        let count = self.runner.running_count();
        let observed = ObservedWorkload {
            name: self.spec.name.clone(),
            running: u32::try_from(count).expect("process count exceeds u32"),
        };
        let actions = plan(&self.spec, &observed);

        for action in actions {
            if !keep_alive.load(Ordering::SeqCst) {
                break;
            }

            match action {
                Action::Start { workload_name } => {
                    if Instant::now() < self.retry_at {
                        break;
                    }

                    println!("Starting workload: {}", workload_name);
                    if let Err(error) = self.runner.start(&self.spec) {
                        eprintln!("Failed to start {}: {}", workload_name, error);
                        self.retry_at = Instant::now() + Duration::from_secs(5);
                        break;
                    }
                }
                Action::Stop { workload_name } => {
                    println!("Stopping one copy of {}", workload_name);
                    self.runner.stop_one();
                }
            }
        }
    }

    fn status(&mut self) -> StatusResponse {
        StatusResponse{workload_name: Some(self.spec.name.clone()), desired: self.spec.replicas, running: self.runner.running_count()}
    }

    fn stop(&mut self) -> StatusResponse {
        self.spec.replicas = 0;
        self.runner.stop_all();
        self.status()
    }

    fn scale(&mut self, replicas: u32) -> Result<StatusResponse, String> {
        if replicas > MAX_REPLICAS {
            return Err(format!("Replica count cannot exceed {}", MAX_REPLICAS));
        }
        self.spec.replicas = replicas;
        Ok(self.status())
    }
}

pub fn allow_connection(address: &str, desired: WorkloadSpec, files: &TlsFiles, allow_remote: bool) -> io::Result<()> {
    let config = Arc::new(tls::server_config(files)?);
    serve(address, desired, config, allow_remote)
}

pub fn allow_device_connection(address: &str, desired: WorkloadSpec, identity: &DeviceIdentity, store: &TrustStore, allow_remote: bool) -> io::Result<()> {
    let config = Arc::new(device_tls::server_config(identity, store)?);
    serve(address, desired, config, allow_remote)
}

fn serve(address: &str, desired: WorkloadSpec, config: Arc<ServerConfig>, allow_remote: bool) -> io::Result<()> {
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

    let listener = TcpListener::bind(listen_address)?;
    listener.set_nonblocking(true)?;
    let keep_alive = Arc::new(AtomicBool::new(true));
    let handler_keep_alive = Arc::clone(&keep_alive);
    ctrlc::set_handler(move || handler_keep_alive.store(false, Ordering::SeqCst))
        .map_err(io::Error::other)?;
    println!("Listening on: {}", address);
    println!("Agent workload: {:?}", desired);

    //todo: run several apps
    let mut workload = ManagedWorkload::new(desired);

    while keep_alive.load(Ordering::SeqCst) {
        workload.tick(&keep_alive);
        match listener.accept() {
            Ok((stream, peer_address)) => {
                println!("Connected: {}", peer_address);
                let result = tls::accept(stream, Arc::clone(&config)).and_then(|mut stream| {
                    let result =
                        handle_client(&mut stream, &mut workload, &expected_token);
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                    result
                });
                if let Err(e) = result {
                    eprintln!("error handling client {}: {}", peer_address, e);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // no client yet
            }
            Err(e) => {
                eprintln!("connection failed: {}", e);
            }
        }
        // pause so we do not waste cpu
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    workload.runner.stop_all();
    Ok(())
}

#[cfg(test)]
mod tests;

fn send_reply(writer: &mut ServerStream, reply: &Response) -> io::Result<()> {
    let json = serde_json::to_string(reply)?;
    writer.write_all(json.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn handle_client(stream: &mut ServerStream, workload: &mut ManagedWorkload, expected_token: &str) -> io::Result<()> {
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

    let reply = match authenticated.request {
        Request::Status => Response::Status { status: workload.status() },
        Request::Stop => Response::Status { status: workload.stop() },
        Request::Scale { replicas } => match workload.scale(replicas) {
            Ok(status) => Response::Status { status },
            Err(message) => Response::Error { message },
        },
    };
    send_reply(stream, &reply)?;

    Ok(())
}
