//! Launches local processes and checks whether they are still running.

use crate::workload::WorkloadSpec;
use std::process::Child;
use std::process::Command;

/// Stores handles to processes launched by Fleet.
///
/// Finished handles are removed when `running_count` checks the list.
pub struct ProcessRunner {
    /// Handles used to inspect the launched processes.
    children: Vec<Child>,
}

impl ProcessRunner {
    /// Creates an empty runner without starting any processes.
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    /// Starts one copy of the workload and stores its process handle.
    /// The child does not inherit Fleet's control token.
    ///
    /// # Panics
    /// Panics if the program cannot be launched.
    pub fn start(&mut self, workload: &WorkloadSpec) {
        let child = Command::new(&workload.program)
            .args(&workload.args)
            .env_remove("FLEET_TOKEN")
            .spawn()
            .expect("failed to start workload");

        println!("Started process: {}", child.id());

        self.children.push(child);
    }

    /// Checks processes without waiting and removes handles for exited processes.
    ///
    /// Returns the number of handles whose processes were observed still running.
    ///
    /// # Panics
    /// Panics if a process status check fails.
    pub fn running_count(&mut self) -> usize {
        self.children.retain_mut(|child| {
            let status = child.try_wait().expect("failed to check process");

            match status {
                None => true,
                Some(_) => false,
            }
        });
        self.children.len()
    }

    pub fn stop_all(&mut self) {
        for child in &mut self.children {
            let status = child.try_wait().expect("failed to check process");

            match status {
                None => {
                    let _ = child.kill().expect("failed to stop process");
                }
                Some(_) => {}
            }

            let _ = child.wait().expect("failed to wait for process");
        }
        self.children.clear();
    }

    /// Force-stops one tracked process and collects its exit status.
    /// Does nothing when no handles remain.
    ///
    /// # Panics
    /// Panics if stopping or waiting for the process fails.
    pub fn stop_one(&mut self) {
        if let Some(mut child) = self.children.pop() {
            child.kill().expect("failed to stop process");
            child.wait().expect("failed to wait for process");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_one_copy_and_handles_an_empty_runner() {
        let workload = WorkloadSpec {
            name: String::from("test"),
            replicas: 2,
            program: String::from("/bin/sleep"),
            args: vec![String::from("60")],
        };
        let mut runner = ProcessRunner::new();
        runner.start(&workload);
        runner.start(&workload);
        runner.stop_one();
        let remaining = runner.running_count();
        runner.stop_all();
        assert_eq!(remaining, 1);
        runner.stop_one();
        assert_eq!(runner.running_count(), 0);
    }

    #[test]
    fn stops_all_started_processes() {
        let workload = WorkloadSpec {
            name: String::from("test"),
            replicas: 1,
            program: String::from("/bin/sleep"),
            args: vec![String::from("60")],
        };

        let mut runner = ProcessRunner::new();
        runner.start(&workload);

        assert_eq!(runner.running_count(), 1);

        runner.stop_all();

        assert_eq!(runner.running_count(), 0);
    }
}
