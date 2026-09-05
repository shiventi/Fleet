mod workload;

use std::process::Command;
use workload::WorkloadSpec;

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

fn main() {
    let desired = WorkloadSpec {
        name: String::from("example-api"),
        replicas: 3,
        program: String::from("/bin/sleep"),
        args: vec![String::from("10")],
    };

    let observed = ObservedWorkload {
        name: String::from("example-api"),
        running: 1,
    };

    let actions = plan(&desired, &observed);

    println!("Planned actions: {actions:#?}");

    let mut child = Command::new(&desired.program)
        .args(&desired.args)
        .spawn()
        .expect("failed to start sleep");
    
    println!("Started process: {}", child.id());
    
    loop {
        let status = child.try_wait().expect("failed to wait for sleep");
        match status {
            Some(exit_status) => {
                println!("Process exited: {exit_status:?}");
                child = Command::new(&desired.program).args(&desired.args).spawn().expect("failed to start sleep");
                println!("Restarted process: {}", child.id())
            },
            None => println!("Process still running.")
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
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
