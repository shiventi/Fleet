//! A replica planner and a basic local process supervisor.

mod agent;
mod cli;
mod client;
mod protocol;
mod runner;
mod workload;

use clap::Parser;
use cli::{Cli, Commands};
use runner::ProcessRunner;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use workload::WorkloadSpec;

/// The reported number of running copies of a workload.
#[derive(Debug)]
struct ObservedWorkload {
    /// The workload this observation belongs to.
    name: String,
    /// The number of copies observed alive during this check.
    running: u32,
}

/// A requested change to a workload's running copies.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Requests one additional copy of the named workload.
    Start { workload_name: String },
    /// Requests one fewer copy; execution is not implemented yet.
    Stop { workload_name: String },
}

/// Decides what to start or stop without running anything.
///
/// # Panics
/// Panics if the desired and observed workload names differ.
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

/// Repeatedly observes the supplied workload and starts missing copies.
fn supervise(desired: WorkloadSpec) {
    let mut runner = ProcessRunner::new();
    let keep_alive = Arc::new(AtomicBool::new(true));
    let handler_keep_alive = Arc::clone(&keep_alive);

    ctrlc::set_handler(move || handler_keep_alive.store(false, Ordering::SeqCst))
        .expect("failed to register the ctrl+c handler");

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
                    println!("Starting workload: {}", workload_name);
                    runner.start(&desired);
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

/// Reads and validates a JSON manifest without starting any processes.
///
/// # Errors
/// Returns an error if the file cannot be read, deserialized, or validated.
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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Agent { listen, manifest } => {
            let desired = match load_manifest(&manifest) {
                Ok(workload) => workload,
                Err(error) => {
                    eprintln!("Error: {}", error);
                    std::process::exit(1);
                }
            };
            if let Err(error) = agent::allow_connection(&listen, desired) {
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

            match client::status(&args.address) {
                Ok(()) => {
                    println!("Connection successful!");
                }
                Err(e) => {
                    eprintln!("Failed to get status: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Stop { address } => {
            if let Err(error) = client::stop(&address) {
                eprintln!("Failed to stop workload: {}", error);
                std::process::exit(1);
            }
        }

        Commands::Scale { address, replicas } => {
            if let Err(error) = client::scale(&address, replicas) {
                eprintln!("Failed to scale workload: {}", error);
                std::process::exit(1);
            }
        }
    }
}

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
