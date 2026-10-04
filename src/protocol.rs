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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
/// A client command, sent as JSON followed by a newline.
pub enum Request {
    /// Ask for the current workload and counts.
    Status,
    /// Set the target to zero and stop all copies.
    Stop,
    /// Change the target; the agent adjusts processes on its next loop.
    Scale { replicas: u32 },
}
