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
    ///
    /// # Panics
    /// Panics if the program cannot be launched.
    pub fn start(&mut self, workload: &WorkloadSpec) {
        let child = Command::new(&workload.program)
            .args(&workload.args)
            .spawn()
            .expect("failed to start sleep");

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
}
