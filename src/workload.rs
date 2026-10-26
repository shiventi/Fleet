use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_REPLICAS: u32 = 32;

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkloadSpec {
    pub name: String,
    pub replicas: u32,
    pub program: String,
    pub args: Vec<String>,
}

impl WorkloadSpec {
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

#[derive(Debug, Deserialize)]
pub struct AgentManifest {
    pub workloads: Vec<WorkloadSpec>,
}

impl AgentManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.workloads.is_empty() {
            return Err("Manifest must contain at least one workload".to_string());
        }

        // each app needs its own name
        let mut hash_set = HashSet::new();

        for workload in &self.workloads {
            workload.validate()?;
            if !hash_set.insert(workload.name.as_str()) {
                return Err(format!("Duplicate app name {} not allowed", workload.name));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
