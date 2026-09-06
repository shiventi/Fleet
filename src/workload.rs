use serde::Deserialize;

/// Describes the program and how many copies should run.
#[derive(Debug, Deserialize)]
pub struct WorkloadSpec {
    /// The workload's name.
    pub name: String,
    /// The desired number of running copies.
    pub replicas: u32,
    /// The executable to launch.
    pub program: String,
    /// Separate arguments passed to the executable.
    pub args: Vec<String>,
}
