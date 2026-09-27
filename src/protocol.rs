//! Shared types for messages between the client and agent.

use serde::{Deserialize, Serialize};

/// The agent's status, sent as JSON on one line.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// The managed workload's name, or `None` when there is no workload.
    pub workload_name: Option<String>,
    /// The requested number of running copies.
    pub desired: u32,
    /// The number of copies observed running.
    pub running: usize,
}
