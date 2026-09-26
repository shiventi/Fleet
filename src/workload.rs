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

impl WorkloadSpec {
    /// Checks that the workload name and executable are not blank.
    ///
    /// Zero replicas and empty argument lists are allowed.
    /// This does not check whether the executable exists or can run.
    ///
    /// # Errors
    /// Returns a message if the name or executable contains only whitespace or is empty.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Workload name cannot be blank".to_string());
        }
        if self.program.trim().is_empty() {
            return Err("Workload program cannot be blank".to_string());
        }

        Ok(())
    }
}
