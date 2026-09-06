//! A replica planner and a basic local process supervisor.

mod cli;
mod runner;
mod workload;

use clap::Parser;
use cli::{Cli, Commands};
use runner::ProcessRunner;
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

/// Repeatedly observes the demo workload and starts missing copies.
fn supervise(desired: WorkloadSpec) {
    let mut runner = ProcessRunner::new();

    loop {
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
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { manifest } => {
            let contents = std::fs::read_to_string(&manifest).expect("failed to read manifest");
            let desired: WorkloadSpec =
                serde_json::from_str(&contents).expect("invalid workload manifest");
            println!("Desired: {:#?}", desired);
            supervise(desired);
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
