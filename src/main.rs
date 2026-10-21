#![doc = include_str!("../docs/fleet.txt")]

#[doc = include_str!("../docs/agent.txt")]
mod agent;
#[doc = include_str!("../docs/cli.txt")]
mod cli;
#[doc = include_str!("../docs/client.txt")]
mod client;
#[doc = include_str!("../docs/controller.txt")]
mod controller;
#[doc = include_str!("../docs/dashboard.txt")]
mod dashboard;
#[doc = include_str!("../docs/identity.txt")]
mod identity;
#[doc = include_str!("../docs/protocol.txt")]
mod protocol;
#[doc = include_str!("../docs/runner.txt")]
mod runner;
#[doc = include_str!("../docs/tls.txt")]
mod tls;
#[doc = include_str!("../docs/workload.txt")]
mod workload;
#[doc = include_str!("../docs/trust.txt")]
mod trust;
#[doc = include_str!("../docs/device_tls.txt")]
mod device_tls;

use clap::Parser;
use cli::{Cli, Commands};
use controller::ClusterSpec;
use runner::ProcessRunner;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use workload::{AgentManifest, WorkloadSpec};

#[derive(Debug)]
struct ObservedWorkload {
    name: String,
    running: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Start { workload_name: String },
    Stop { workload_name: String },
}

fn plan(desired: &WorkloadSpec, observed: &ObservedWorkload) -> Vec<Action> {
    assert_eq!(
        desired.name, observed.name,
        "desired and observed workloads have to have same name"
    );

    if observed.running < desired.replicas {
        let missing = desired.replicas - observed.running;

        (0..missing)
            .map(|_| Action::Start {
                workload_name: desired.name.clone(),
            })
            .collect()
    } else {
        let extra = observed.running - desired.replicas;

        (0..extra)
            .map(|_| Action::Stop {
                workload_name: desired.name.clone(),
            })
            .collect()
    }
}

fn supervise(desired: WorkloadSpec) {
    let mut runner = ProcessRunner::new();
    let keep_alive = Arc::new(AtomicBool::new(true));
    let handler_keep_alive = Arc::clone(&keep_alive);

    ctrlc::set_handler(move || handler_keep_alive.store(false, Ordering::SeqCst))
        .expect("failed to register the ctrl+c handler");

    let mut retry_at = Instant::now();

    while keep_alive.load(Ordering::SeqCst) {
        let count = runner.running_count();

        let observed = ObservedWorkload {
            name: desired.name.clone(),
            running: u32::try_from(count).expect("process count exceeds u32"),
        };

        let actions = plan(&desired, &observed);

        println!("Planned actions: {actions:#?}");

        println!("Running count: {}", count);

        for act in actions {
            match act {
                Action::Start { workload_name } => {
                    if Instant::now() < retry_at {
                        break;
                    }
                    println!("Starting workload: {}", workload_name);
                    if let Err(error) = runner.start(&desired) {
                        eprintln!("Failed to start {}: {}", workload_name, error);
                        retry_at = Instant::now() + Duration::from_secs(5);
                        break;
                    }
                }
                Action::Stop { workload_name } => {
                    println!("Stopping isn't implemented yet for {}", workload_name);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    runner.stop_all();
}

fn load_manifest(path: &std::path::Path) -> Result<WorkloadSpec, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;

    let desired: WorkloadSpec = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse JSON in '{}': {}", path.display(), e))?;

    desired
        .validate()
        .map_err(|error| format!("Invalid workload manifest '{}': {}", path.display(), error))?;

    Ok(desired)
}

fn load_agent_manifest(path: &std::path::Path) -> Result<AgentManifest, String> {
    let contents = std::fs::read_to_string(path).map_err(|err| format!("Failed to read file '{}': {}", path.display(), err))?;
    let value: serde_json::Value = serde_json::from_str(&contents).map_err(|err| format!("Failed to parse JSON in '{}': {}", path.display(), err))?;

    let desired: AgentManifest = if value.get("workloads").is_some() {
        serde_json::from_value(value).map_err(|err| format!("Failed to parse JSON in '{}': {}", path.display(), err))?
    } else {
        let spec: WorkloadSpec = serde_json::from_value(value).map_err(|err| format!("Failed to parse JSON in '{}': {}", path.display(), err))?;

        AgentManifest {
            workloads: vec![spec],
        }
    };
    desired.validate().map_err(|err| format!("Invalid agent manifest '{}': {}", path.display(), err))?;

    Ok(desired)
}

fn load_cluster(path: &std::path::Path) -> Result<ClusterSpec, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|err| format!("Failed to read cluster file: {}", err))?;
    let cluster: ClusterSpec =
        serde_json::from_str(&contents).map_err(|err| format!("Invalid cluster JSON: {}", err))?;

    cluster.validate()?;
    Ok(cluster)
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Identity { fingerprint } => match identity::DeviceIdentity::load_default() {
            Ok(identity) => {
                if fingerprint {
                    match identity::fingerprint(&identity.public_key_pem()) {
                        Ok(out) => println!("{}", out),
                        Err(err) => {
                            eprintln!("Identity error: {}", err);
                            std::process::exit(1);
                        }
                    }
                } else {
                    print!("{}", identity.public_key_pem());
                }
            }
            Err(error) => {
                eprintln!("Identity error: {}", error);
                std::process::exit(1);
            }
        },
        Commands::Peers { command } => {
            if let Err(error) = trust::run(command) {
                eprintln!("Peer error: {}", error);
                std::process::exit(1);
            }
        },
        Commands::Agent {
            listen,
            manifest,
            tls,
            allow_remote,
        } => {
            let desired = match load_agent_manifest(&manifest) {
                Ok(workload) => workload,
                Err(error) => {
                    eprintln!("Error: {}", error);
                    std::process::exit(1);
                }
            };
            if let Err(error) = agent::allow_connection(&listen, desired, &tls, allow_remote) {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        }
        Commands::Run { manifest } => {
            let desired = match load_manifest(&manifest) {
                Ok(desired) => desired,
                Err(error) => {
                    eprintln!("Error: {}", error);
                    std::process::exit(1);
                }
            };
            println!("Desired: {:#?}", desired);
            supervise(desired);
        }
        Commands::Validate { manifest } => match load_manifest(&manifest) {
            Ok(_) => {
                println!("Manifest is valid.");
            }
            Err(error) => {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        },
        Commands::Status(args) => {
            println!("Will connect to: {}", args.address);

            match client::status(&args.address, &args.server_name, &args.tls) {
                Ok(()) => {
                    println!("Connection successful!");
                }
                Err(e) => {
                    eprintln!("Failed to get status: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Stop {
            address,
            server_name,
            tls,
        } => {
            if let Err(error) = client::stop(&address, &server_name, &tls) {
                eprintln!("Failed to stop workload: {}", error);
                std::process::exit(1);
            }
        }

        Commands::Scale {
            address,
            replicas,
            server_name,
            tls,
        } => {
            if let Err(error) = client::scale(&address, replicas, &server_name, &tls) {
                eprintln!("Failed to scale workload: {}", error);
                std::process::exit(1);
            }
        }

        Commands::Watch { manifest, tls } => {
            let cluster = match load_cluster(&manifest) {
                Ok(cluster) => cluster,
                Err(error) => {
                    eprintln!("Cluster error: {}", error);
                    std::process::exit(1);
                }
            };

            if let Err(error) = controller::watch(&cluster, &tls) {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        }

        Commands::WatchDevice { manifest } => {
            let cluster = match load_cluster(&manifest) {
                Ok(cluster) => cluster,
                Err(error) => {
                    eprintln!("Cluster error: {}", error);
                    std::process::exit(1);
                }
            };
            if let Err(error) = controller::watch_device(&cluster) {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        }

        Commands::Dashboard { manifest, tls } => {
            let cluster = match load_cluster(&manifest) {
                Ok(cluster) => cluster,
                Err(error) => {
                    eprintln!("Cluster error: {}", error);
                    std::process::exit(1);
                }
            };

            if let Err(error) = dashboard::run(&cluster, &tls) {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        }

        Commands::DashboardDevice { manifest } => {
            let cluster = match load_cluster(&manifest) {
                Ok(cluster) => cluster,
                Err(error) => {
                    eprintln!("Cluster error: {}", error);
                    std::process::exit(1);
                }
            };

            if let Err(error) = dashboard::run_device(&cluster) {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            }
        }

        Commands::AgentDevice { listen, manifest, allow_remote } => {
            let desired = match load_agent_manifest(&manifest) {
                Ok(workload) => workload,
                Err(error) => {
                    eprintln!("Manifest error: {}", error);
                    std::process::exit(1);
                }
            };

            let identity = match identity::DeviceIdentity::load_default() {
                Ok(identity) => identity,
                Err(error) => {
                    eprintln!("Identity error: {}", error);
                    std::process::exit(1);
                }
            };
            let directory = match identity::default_directory() {
                Ok(directory) => directory,
                Err(error) => {
                    eprintln!("Config directory error: {}", error);
                    std::process::exit(1);
                }
            };
            let peer_list = match trust::TrustStore::load(&directory.join("trusted-peers.json")) {
                Ok(store) => store,
                Err(error) => {
                    eprintln!("Peer list error: {}", error);
                    std::process::exit(1);
                }
            };
            if let Err(error) = agent::allow_device_connection(&listen, desired, &identity, &peer_list, allow_remote) {
                eprintln!("Agent error: {}", error);
                std::process::exit(1);
            }
        }

        Commands::StatusDevice { address, peer } => {
            let client = match client::FleetClient::for_peer(&peer) {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("Client error: {}", error);
                    std::process::exit(1);
                }
            };
            match client.fetch_status(&address, "localhost") {
                Ok(status) => {
                    if let Some(name) = status.workload_name {
                        println!("Workload: {}", name);
                    }
                    println!("Desired copies: {}", status.desired);
                    println!("Running copies: {}", status.running);
                }
                Err(error) => {
                    eprintln!("Failed to get status: {}", error);
                    std::process::exit(1);
                }
            }
        }

        Commands::StopDevice { address, peer } => {
            let client = match client::FleetClient::for_peer(&peer) {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("Client error: {}", error);
                    std::process::exit(1);
                }
            };
            if let Err(error) = client.stop(&address, "localhost") {
                eprintln!("Failed to stop workload: {}", error);
                std::process::exit(1);
            }
        }

        Commands::ScaleDevice { address, peer, replicas } => {
            let client = match client::FleetClient::for_peer(&peer) {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("Client error: {}", error);
                    std::process::exit(1);
                }
            };
            match client.scale(&address, replicas, "localhost") {
                Ok(status) => {
                    println!("Desired copies: {}", status.desired);
                    println!("Running copies: {}", status.running);
                }
                Err(error) => {
                    eprintln!("Failed to scale workload: {}", error);
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod manifest_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn actions_for(desired: u32, running: u32) -> Vec<Action> {
        let workload = WorkloadSpec {
            name: String::from("test"),
            replicas: desired,
            program: String::from("/bin/sleep"),
            args: vec![String::from("10")],
        };

        let observed = ObservedWorkload {
            name: String::from("test"),
            running,
        };

        plan(&workload, &observed)
    }

    #[test]
    fn starts_one_missing_replica() {
        assert_eq!(
            actions_for(1, 0),
            vec![Action::Start {
                workload_name: String::from("test"),
            }]
        );
    }

    #[test]
    fn does_nothing_when_state_matches() {
        assert_eq!(actions_for(1, 1), Vec::<Action>::new());
    }

    #[test]
    fn stops_one_extra_replica() {
        assert_eq!(
            actions_for(1, 2),
            vec![Action::Stop {
                workload_name: String::from("test"),
            }]
        );
    }

    #[test]
    fn starts_multiple_missing_replicas() {
        let expected: Vec<_> = (0..2)
            .map(|_| Action::Start {
                workload_name: String::from("test"),
            })
            .collect();
        assert_eq!(actions_for(3, 1), expected);
    }

    #[test]
    fn stops_all_unwanted_replicas() {
        let expected: Vec<_> = (0..3)
            .map(|_| Action::Stop {
                workload_name: String::from("test"),
            })
            .collect();
        assert_eq!(actions_for(0, 3), expected);
    }

    #[test]
    fn does_nothing_when_no_replicas_are_requested_or_running() {
        assert!(actions_for(0, 0).is_empty());
    }
}
