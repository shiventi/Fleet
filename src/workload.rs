use serde::Deserialize;

/// The largest copy count allowed in a manifest or scale request.
pub const MAX_REPLICAS: u32 = 32;

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
    /// Checks the name, executable, and copy count.
    ///
    /// Zero replicas and empty argument lists are allowed.
    /// This does not check whether the executable exists or can run.
    ///
    /// # Errors
    /// Returns a message if the name or executable is blank, or the count is too high.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Workload name cannot be blank".to_string());
        }
        if self.program.trim().is_empty() {
            return Err("Workload program cannot be blank".to_string());
        }
        if self.replicas > MAX_REPLICAS {
            return Err(format!("Replica count cannot exceed {}", MAX_REPLICAS));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_replica_limits() {
        let mut workload = WorkloadSpec {
            name: String::from("test"),
            program: String::from("/bin/sleep"),
            args: Vec::new(),
            replicas: 0,
        };
        assert!(workload.validate().is_ok());
        workload.replicas = MAX_REPLICAS;
        assert!(workload.validate().is_ok());
        workload.replicas = MAX_REPLICAS + 1;
        assert!(workload.validate().is_err());
    }
}
