//! Shared types for JSON messages between the client and agent.
//!
//! Requests include a shared token. Each message ends with a newline.
//! TCP does not encrypt the token; remote connections need an encrypted tunnel.

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
/// A client command carried inside an authenticated request.
pub enum Request {
    /// Ask for the current workload and counts.
    Status,
    /// Set the target to zero and stop all copies.
    Stop,
    /// Change the target; the agent adjusts processes on its next loop.
    Scale { replicas: u32 },
}

/// An agent reply: current status or a reason the request was rejected.
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// The workload and current counts.
    Status { status: StatusResponse },
    /// A readable error message.
    Error { message: String },
}

/// A command and the shared token needed to run it.
///
/// Debug printing is not derived to avoid accidentally logging the token.
#[derive(Serialize, Deserialize)]
pub struct AuthenticatedRequest {
    /// The secret read from `FLEET_TOKEN`. Do not log it.
    pub token: String,
    /// The command to handle after checking the token.
    pub request: Request,
}
